use std::fs::{self, File};
use std::io::copy;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use reqwest::blocking::{Body, Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize};

use crate::settings::SupabaseSettings;

const STORAGE_BUCKET: &str = "fast-explorer-transfers";
const POLL_SECONDS: u64 = 5;
const MAX_CLIPBOARD_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub device_key: String,
    pub name: String,
    pub platform: String,
    #[serde(default)]
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: String,
    #[serde(default)]
    pub sender_device_id: String,
    pub receiver_device_id: String,
    pub kind: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub object_path: String,
    #[serde(default)]
    pub clipboard_text: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug)]
pub struct UiState {
    pub settings: SupabaseSettings,
    pub email_input: String,
    pub otp_input: String,
    pub status: String,
    pub signed_in: bool,
    pub user_email: String,
    pub devices: Vec<Device>,
    pub inbox: Vec<InboxItem>,
    pub push_token: String,
    pub sender: Option<xilem::tokio::sync::mpsc::UnboundedSender<Command>>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            settings: SupabaseSettings::default(),
            email_input: String::new(),
            otp_input: String::new(),
            status: "Supabase is not configured".to_owned(),
            signed_in: false,
            user_email: String::new(),
            devices: Vec::new(),
            inbox: Vec::new(),
            push_token: String::new(),
            sender: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Configure {
        settings: SupabaseSettings,
        session_path: Option<PathBuf>,
    },
    RequestOtp {
        email: String,
    },
    VerifyOtp {
        email: String,
        token: String,
    },
    SignOut,
    RefreshDevices,
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    UpdatePushToken {
        token: String,
    },
    SendClipboard {
        receiver_device_id: String,
        text: String,
    },
    SendFile {
        receiver_device_id: String,
        path: PathBuf,
    },
    ReceiveFile {
        item: InboxItem,
    },
    AckTransfer {
        transfer_id: String,
    },
}

#[derive(Debug, Clone)]
pub enum Event {
    AuthState {
        signed_in: bool,
        email: String,
        status: String,
    },
    OtpSent(String),
    Devices(Result<Vec<Device>, String>),
    Inbox(Result<Vec<InboxItem>, String>),
    PushTokenUpdated {
        token: String,
        result: Result<(), String>,
    },
    Sent(Result<String, String>),
    FileReceived {
        transfer_id: String,
        result: Result<PathBuf, String>,
    },
    TransferAcked {
        transfer_id: String,
        result: Result<(), String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Session {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    user_id: String,
    email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSession {
    project_url: String,
    session: Session,
}

#[derive(Debug, Deserialize)]
struct AuthUser {
    id: String,
    #[serde(default)]
    email: String,
}

#[derive(Debug, Deserialize)]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
    user: AuthUser,
}

#[derive(Debug, Serialize)]
struct DeviceUpsert<'a> {
    user_id: &'a str,
    device_key: &'a str,
    name: &'a str,
    platform: &'a str,
}

#[derive(Debug, Serialize)]
struct TransferInsert<'a> {
    id: &'a str,
    user_id: &'a str,
    sender_device_id: &'a str,
    receiver_device_id: &'a str,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    object_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clipboard_text: Option<&'a str>,
}

#[derive(Debug)]
struct Engine {
    client: Client,
    settings: SupabaseSettings,
    session_path: Option<PathBuf>,
    session: Option<Session>,
    local_device_id: Option<String>,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(90))
                .build()
                .expect("Supabase HTTP client"),
            settings: SupabaseSettings::default(),
            session_path: None,
            session: None,
            local_device_id: None,
        }
    }
}

impl Engine {
    fn configure(&mut self, settings: SupabaseSettings, session_path: Option<PathBuf>) -> Event {
        let project_changed = normalized_project_url(&self.settings.project_url)
            != normalized_project_url(&settings.project_url);
        self.settings = settings;
        self.session_path = session_path;
        self.local_device_id = None;
        if project_changed {
            self.session = None;
        }
        if !self.settings.is_configured() {
            self.session = None;
            return self.auth_event(
                false,
                "Supabase project URL and publishable key are required",
            );
        }
        if self.session.is_none() {
            self.session = self.load_session();
        }
        let signed_in = self.session.is_some();
        let status = if signed_in {
            "Signed in; registering this device"
        } else {
            "Configured. Request an email OTP to sign in"
        };
        self.auth_event(signed_in, status)
    }

    fn auth_event(&self, signed_in: bool, status: impl Into<String>) -> Event {
        Event::AuthState {
            signed_in,
            email: self
                .session
                .as_ref()
                .map(|session| session.email.clone())
                .unwrap_or_default(),
            status: status.into(),
        }
    }

    fn request_otp(&self, email: &str) -> Result<(), String> {
        self.ensure_configured()?;
        let email = email.trim();
        if email.is_empty() || !email.contains('@') {
            return Err("Enter a valid email address".to_owned());
        }
        let response = self
            .client
            .post(self.endpoint("auth/v1/otp")?)
            .header("apikey", self.settings.publishable_key.trim())
            .json(&serde_json::json!({"email": email, "create_user": true}))
            .send()
            .map_err(|error| format!("Cannot request Supabase OTP: {error}"))?;
        expect_success(response, "Supabase OTP request")?;
        Ok(())
    }

    fn verify_otp(&mut self, email: &str, token: &str) -> Result<(), String> {
        self.ensure_configured()?;
        let token = token.trim();
        if token.is_empty() {
            return Err("Enter the OTP from your email".to_owned());
        }
        let response = self
            .client
            .post(self.endpoint("auth/v1/verify")?)
            .header("apikey", self.settings.publishable_key.trim())
            .json(&serde_json::json!({
                "email": email.trim(),
                "token": token,
                "type": "email"
            }))
            .send()
            .map_err(|error| format!("Cannot verify Supabase OTP: {error}"))?;
        let auth: AuthResponse = response_json(response, "Supabase OTP verification")?;
        self.session = Some(session_from_auth(auth));
        self.local_device_id = None;
        self.save_session()?;
        Ok(())
    }

    fn sign_out(&mut self) -> Result<(), String> {
        if self.local_device_id.is_some() {
            let _ = self.set_push_token(None);
        }
        if let Some(session) = self.session.as_ref() {
            let response = self
                .client
                .post(self.endpoint("auth/v1/logout")?)
                .header("apikey", self.settings.publishable_key.trim())
                .bearer_auth(&session.access_token)
                .send();
            if let Ok(response) = response {
                let _ = expect_success(response, "Supabase sign out");
            }
        }
        self.session = None;
        self.local_device_id = None;
        if let Some(path) = self.session_path.as_ref() {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    fn ensure_configured(&self) -> Result<(), String> {
        if !self.settings.is_configured() {
            return Err("Configure the Supabase project URL and publishable key first".to_owned());
        }
        if is_privileged_api_key(&self.settings.publishable_key) {
            return Err(
                "Use a Supabase publishable/anon key here; secret/service_role keys are not allowed"
                    .to_owned(),
            );
        }
        let url = reqwest::Url::parse(&normalized_project_url(&self.settings.project_url))
            .map_err(|error| format!("Invalid Supabase project URL: {error}"))?;
        let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
        let loopback_host = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        let local_http = url.scheme() == "http" && loopback_host;
        if url.scheme() != "https" && !local_http {
            return Err(
                "Supabase project URL must use HTTPS (HTTP is only allowed for localhost)"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn ensure_session(&mut self) -> Result<Session, String> {
        self.ensure_configured()?;
        let needs_refresh = self
            .session
            .as_ref()
            .is_some_and(|session| session.expires_at <= unix_seconds().saturating_add(60));
        if needs_refresh {
            self.refresh_session()?;
        }
        self.session
            .clone()
            .ok_or_else(|| "Sign in to Supabase first".to_owned())
    }

    fn refresh_session(&mut self) -> Result<(), String> {
        let refresh_token = self
            .session
            .as_ref()
            .map(|session| session.refresh_token.clone())
            .ok_or_else(|| "No Supabase refresh token is available".to_owned())?;
        let response = self
            .client
            .post(self.endpoint("auth/v1/token?grant_type=refresh_token")?)
            .header("apikey", self.settings.publishable_key.trim())
            .json(&serde_json::json!({"refresh_token": refresh_token}))
            .send()
            .map_err(|error| format!("Cannot refresh Supabase session: {error}"))?;
        let auth: AuthResponse = response_json(response, "Supabase session refresh")?;
        self.session = Some(session_from_auth(auth));
        self.save_session()?;
        Ok(())
    }

    fn register_device(&mut self) -> Result<Device, String> {
        let session = self.ensure_session()?;
        let device_key = self.settings.device_id.trim();
        let device_name = self.settings.device_name.trim();
        if device_key.is_empty() || device_name.is_empty() {
            return Err("This device is missing its local device ID or name".to_owned());
        }
        let body = DeviceUpsert {
            user_id: &session.user_id,
            device_key,
            name: device_name,
            platform: platform_name(),
        };
        let response = self
            .auth_request(
                self.client.post(
                    self.endpoint("rest/v1/fast_explorer_devices?on_conflict=user_id,device_key")?,
                ),
                &session,
            )
            .header(
                "Prefer",
                "resolution=merge-duplicates,return=representation",
            )
            .json(&body)
            .send()
            .map_err(|error| format!("Cannot register Supabase device: {error}"))?;
        let mut devices: Vec<Device> = response_json(response, "Supabase device registration")?;
        let device = devices
            .pop()
            .ok_or_else(|| "Supabase did not return the registered device".to_owned())?;
        self.local_device_id = Some(device.id.clone());
        Ok(device)
    }

    fn update_push_token(&mut self, token: &str) -> Result<(), String> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(());
        }
        if token.len() > 4096 {
            return Err("FCM token is unexpectedly large".to_owned());
        }
        if self.local_device_id.is_none() {
            self.register_device()?;
        }
        self.set_push_token(Some(token))
    }

    fn set_push_token(&mut self, token: Option<&str>) -> Result<(), String> {
        let session = self.ensure_session()?;
        let device_id = self
            .local_device_id
            .as_ref()
            .ok_or_else(|| "This device is not registered in Supabase".to_owned())?;
        let endpoint = format!("rest/v1/fast_explorer_devices?id=eq.{device_id}");
        let response = self
            .auth_request(self.client.patch(self.endpoint(&endpoint)?), &session)
            .header("Prefer", "return=minimal")
            .json(&serde_json::json!({"push_token": token, "push_provider": token.map(|_| "fcm")}))
            .send()
            .map_err(|error| format!("Cannot update Supabase push token: {error}"))?;
        expect_success(response, "Supabase push token update")
    }

    fn list_devices(&mut self) -> Result<Vec<Device>, String> {
        let session = self.ensure_session()?;
        if self.local_device_id.is_none() {
            self.register_device()?;
        }
        let response = self
            .auth_request(
                self.client.get(self.endpoint(
                    "rest/v1/fast_explorer_devices?select=id,device_key,name,platform,last_seen_at&order=name.asc",
                )?),
                &session,
            )
            .send()
            .map_err(|error| format!("Cannot load Supabase devices: {error}"))?;
        response_json(response, "Supabase device list")
    }

    fn poll_inbox(&mut self) -> Result<Vec<InboxItem>, String> {
        let session = self.ensure_session()?;
        let receiver = self
            .local_device_id
            .clone()
            .map_or_else(|| self.register_device().map(|device| device.id), Ok)?;
        let endpoint = format!(
            "rest/v1/fast_explorer_transfers?select=id,sender_device_id,receiver_device_id,kind,file_name,object_path,clipboard_text,created_at&receiver_device_id=eq.{receiver}&status=eq.pending&order=created_at.asc"
        );
        let response = self
            .auth_request(self.client.get(self.endpoint(&endpoint)?), &session)
            .send()
            .map_err(|error| format!("Cannot poll Supabase inbox: {error}"))?;
        response_json(response, "Supabase inbox")
    }

    fn send_clipboard(&mut self, receiver: &str, text: &str) -> Result<String, String> {
        if text.is_empty() {
            return Err("The system clipboard does not contain text".to_owned());
        }
        if text.len() > MAX_CLIPBOARD_BYTES {
            return Err("Clipboard text is larger than 256 KiB".to_owned());
        }
        let session = self.ensure_session()?;
        let sender = self
            .local_device_id
            .clone()
            .map_or_else(|| self.register_device().map(|device| device.id), Ok)?;
        let transfer_id = uuid::Uuid::new_v4().to_string();
        let insert = TransferInsert {
            id: &transfer_id,
            user_id: &session.user_id,
            sender_device_id: &sender,
            receiver_device_id: receiver,
            kind: "clipboard",
            file_name: None,
            object_path: None,
            clipboard_text: Some(text),
        };
        self.insert_transfer(&session, &insert)?;
        Ok("Clipboard sent".to_owned())
    }

    fn send_file(&mut self, receiver: &str, path: &Path) -> Result<String, String> {
        if !path.is_file() {
            return Err("Select a local file before sending it to a device".to_owned());
        }
        let session = self.ensure_session()?;
        let sender = self
            .local_device_id
            .clone()
            .map_or_else(|| self.register_device().map(|device| device.id), Ok)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "The selected file name is not valid UTF-8".to_owned())?;
        let transfer_id = uuid::Uuid::new_v4().to_string();
        let object_path = format!("{}/{}/{}/payload", session.user_id, receiver, transfer_id);
        self.upload_file(&session, &object_path, path)?;
        let insert = TransferInsert {
            id: &transfer_id,
            user_id: &session.user_id,
            sender_device_id: &sender,
            receiver_device_id: receiver,
            kind: "file",
            file_name: Some(file_name),
            object_path: Some(&object_path),
            clipboard_text: None,
        };
        if let Err(error) = self.insert_transfer(&session, &insert) {
            let _ = self.delete_object(&session, &object_path);
            return Err(error);
        }
        Ok(format!("Sent {file_name}"))
    }

    fn insert_transfer(
        &self,
        session: &Session,
        insert: &TransferInsert<'_>,
    ) -> Result<(), String> {
        let response = self
            .auth_request(
                self.client
                    .post(self.endpoint("rest/v1/fast_explorer_transfers")?),
                session,
            )
            .header("Prefer", "return=minimal")
            .json(insert)
            .send()
            .map_err(|error| format!("Cannot create Supabase transfer: {error}"))?;
        expect_success(response, "Supabase transfer creation")
    }

    fn upload_file(&self, session: &Session, object_path: &str, path: &Path) -> Result<(), String> {
        let file =
            File::open(path).map_err(|error| format!("Cannot open file for upload: {error}"))?;
        let content_length = file
            .metadata()
            .map_err(|error| format!("Cannot read upload file metadata: {error}"))?
            .len();
        let endpoint = format!(
            "storage/v1/object/{STORAGE_BUCKET}/{}",
            encode_object_path(object_path)
        );
        let response = self
            .auth_request(self.client.post(self.endpoint(&endpoint)?), session)
            .header("x-upsert", "false")
            .header("cache-control", "max-age=3600")
            .header("content-type", "application/octet-stream")
            .header("content-length", content_length)
            .body(Body::new(file))
            .send()
            .map_err(|error| format!("Cannot upload file to Supabase Storage: {error}"))?;
        expect_success(response, "Supabase file upload")
    }

    fn receive_file(&mut self, item: &InboxItem) -> Result<PathBuf, String> {
        if item.kind != "file" || item.object_path.is_empty() {
            return Err("This inbox item does not contain a file".to_owned());
        }
        let session = self.ensure_session()?;
        let file_name = safe_received_name(&item.file_name, &item.id);
        let destination_dir = receive_directory()?;
        fs::create_dir_all(&destination_dir)
            .map_err(|error| format!("Cannot create receive folder: {error}"))?;
        let destination = unique_destination(&destination_dir, &file_name);
        let temp = destination.with_extension(format!("part.{}", std::process::id()));
        let endpoint = format!(
            "storage/v1/object/authenticated/{STORAGE_BUCKET}/{}",
            encode_object_path(&item.object_path)
        );
        let mut response = self
            .auth_request(self.client.get(self.endpoint(&endpoint)?), &session)
            .send()
            .map_err(|error| format!("Cannot download Supabase file: {error}"))?;
        if !response.status().is_success() {
            return Err(response_error(response, "Supabase file download"));
        }
        let mut output =
            File::create(&temp).map_err(|error| format!("Cannot create received file: {error}"))?;
        copy(&mut response, &mut output)
            .map_err(|error| format!("Cannot write received file: {error}"))?;
        drop(output);
        fs::rename(&temp, &destination).map_err(|error| {
            let _ = fs::remove_file(&temp);
            format!("Cannot finalize received file: {error}")
        })?;
        if let Err(error) = self.ack_transfer(&item.id) {
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        let _ = self.delete_object(&session, &item.object_path);
        Ok(destination)
    }

    fn ack_transfer(&mut self, transfer_id: &str) -> Result<(), String> {
        let session = self.ensure_session()?;
        let endpoint = format!("rest/v1/fast_explorer_transfers?id=eq.{transfer_id}");
        let response = self
            .auth_request(self.client.delete(self.endpoint(&endpoint)?), &session)
            .header("Prefer", "return=minimal")
            .send()
            .map_err(|error| format!("Cannot acknowledge Supabase transfer: {error}"))?;
        expect_success(response, "Supabase transfer acknowledgement")
    }

    fn delete_object(&self, session: &Session, object_path: &str) -> Result<(), String> {
        let endpoint = format!("storage/v1/object/{STORAGE_BUCKET}");
        let response = self
            .auth_request(self.client.delete(self.endpoint(&endpoint)?), session)
            .json(&serde_json::json!({"prefixes": [object_path]}))
            .send()
            .map_err(|error| format!("Cannot remove Supabase object: {error}"))?;
        expect_success(response, "Supabase object cleanup")
    }

    fn auth_request(&self, request: RequestBuilder, session: &Session) -> RequestBuilder {
        request
            .header("apikey", self.settings.publishable_key.trim())
            .bearer_auth(&session.access_token)
    }

    fn endpoint(&self, path: &str) -> Result<String, String> {
        self.ensure_configured()?;
        Ok(format!(
            "{}/{}",
            normalized_project_url(&self.settings.project_url),
            path.trim_start_matches('/')
        ))
    }

    fn save_session(&self) -> Result<(), String> {
        let (Some(path), Some(session)) = (self.session_path.as_ref(), self.session.as_ref())
        else {
            return Ok(());
        };
        let stored = StoredSession {
            project_url: normalized_project_url(&self.settings.project_url),
            session: session.clone(),
        };
        let parent = path
            .parent()
            .ok_or_else(|| "Supabase session path has no parent".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(
            &temp,
            serde_json::to_vec_pretty(&stored).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        if fs::rename(&temp, path).is_err() {
            let _ = fs::remove_file(path);
            fs::rename(&temp, path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn load_session(&self) -> Option<Session> {
        let path = self.session_path.as_ref()?;
        let text = fs::read_to_string(path).ok()?;
        let stored = serde_json::from_str::<StoredSession>(&text).ok()?;
        (stored.project_url == normalized_project_url(&self.settings.project_url))
            .then_some(stored.session)
    }
}

enum WorkerRequest {
    Command(Command),
    PollInbox,
}

struct WorkerResult {
    events: Vec<Event>,
    poll_completed: bool,
}

fn run_blocking_engine(
    requests: std::sync::mpsc::Receiver<WorkerRequest>,
    results: xilem::tokio::sync::mpsc::UnboundedSender<WorkerResult>,
) {
    // reqwest::blocking owns an internal Tokio runtime. Keep the Client's complete
    // lifecycle on this blocking thread so it is never created or dropped from an
    // async Tokio worker context.
    let mut engine = Engine::default();
    while let Ok(request) = requests.recv() {
        let (events, poll_completed) = match request {
            WorkerRequest::Command(command) => (handle_command(&mut engine, command), false),
            WorkerRequest::PollInbox => {
                let events = if engine.session.is_some() && engine.settings.is_configured() {
                    vec![Event::Inbox(engine.poll_inbox())]
                } else {
                    Vec::new()
                };
                (events, true)
            }
        };
        if results
            .send(WorkerResult {
                events,
                poll_completed,
            })
            .is_err()
        {
            break;
        }
    }
}

pub async fn run_worker(
    proxy: xilem::core::MessageProxy<Event>,
    mut rx: xilem::tokio::sync::mpsc::UnboundedReceiver<Command>,
) {
    let (request_tx, request_rx) = std::sync::mpsc::channel();
    let (result_tx, mut result_rx) = xilem::tokio::sync::mpsc::unbounded_channel();
    let blocking_worker = match std::thread::Builder::new()
        .name("FastExplorer Supabase".to_owned())
        .spawn(move || run_blocking_engine(request_rx, result_tx))
    {
        Ok(worker) => worker,
        Err(error) => {
            let _ = proxy.message(Event::AuthState {
                signed_in: false,
                email: String::new(),
                status: format!("Cannot start Supabase worker: {error}"),
            });
            return;
        }
    };

    let mut interval = xilem::tokio::time::interval(Duration::from_secs(POLL_SECONDS));
    interval.set_missed_tick_behavior(xilem::tokio::time::MissedTickBehavior::Skip);
    let mut poll_pending = false;

    loop {
        xilem::tokio::select! {
            command = rx.recv() => {
                let Some(command) = command else { break; };
                if request_tx.send(WorkerRequest::Command(command)).is_err() {
                    break;
                }
            }
            _ = interval.tick(), if !poll_pending => {
                if request_tx.send(WorkerRequest::PollInbox).is_err() {
                    break;
                }
                poll_pending = true;
            }
            result = result_rx.recv() => {
                let Some(result) = result else { break; };
                if result.poll_completed {
                    poll_pending = false;
                }
                for event in result.events {
                    if proxy.message(event).is_err() {
                        drop(request_tx);
                        return;
                    }
                }
            }
        }
    }

    drop(request_tx);
    // Do not await an in-flight HTTP request during UI teardown. Dropping the
    // JoinHandle detaches the blocking task; the closed request channel makes the
    // engine thread exit (and drop reqwest::blocking::Client there) afterward.
    drop(blocking_worker);
}

fn handle_command(engine: &mut Engine, command: Command) -> Vec<Event> {
    match command {
        Command::Configure {
            settings,
            session_path,
        } => {
            let event = engine.configure(settings, session_path);
            let signed_in = matches!(
                &event,
                Event::AuthState {
                    signed_in: true,
                    ..
                }
            );
            let mut events = vec![event];
            if signed_in {
                let registration = engine.register_device();
                match registration {
                    Ok(_) => {
                        events.push(Event::Devices(engine.list_devices()));
                        events.push(Event::Inbox(engine.poll_inbox()));
                    }
                    Err(error) => events.push(engine.auth_event(true, error)),
                }
            }
            events
        }
        Command::RequestOtp { email } => match engine.request_otp(&email) {
            Ok(()) => vec![Event::OtpSent(
                "OTP sent. Enter the code from your Supabase auth email.".to_owned(),
            )],
            Err(error) => vec![engine.auth_event(false, error)],
        },
        Command::VerifyOtp { email, token } => match engine.verify_otp(&email, &token) {
            Ok(()) => match engine.register_device() {
                Ok(_) => vec![
                    engine.auth_event(true, "Signed in and registered this device"),
                    Event::Devices(engine.list_devices()),
                    Event::Inbox(engine.poll_inbox()),
                ],
                Err(error) => vec![engine.auth_event(
                    true,
                    format!("Signed in, but this device could not be registered: {error}"),
                )],
            },
            Err(error) => vec![engine.auth_event(false, error)],
        },
        Command::SignOut => {
            let result = engine.sign_out();
            vec![
                engine.auth_event(
                    false,
                    result
                        .err()
                        .unwrap_or_else(|| "Signed out of Supabase".to_owned()),
                ),
            ]
        }
        Command::RefreshDevices => vec![Event::Devices(engine.list_devices())],
        Command::UpdatePushToken { token } => vec![Event::PushTokenUpdated {
            token: token.clone(),
            result: engine.update_push_token(&token),
        }],
        Command::SendClipboard {
            receiver_device_id,
            text,
        } => vec![Event::Sent(
            engine.send_clipboard(&receiver_device_id, &text),
        )],
        Command::SendFile {
            receiver_device_id,
            path,
        } => vec![Event::Sent(engine.send_file(&receiver_device_id, &path))],
        Command::ReceiveFile { item } => vec![Event::FileReceived {
            transfer_id: item.id.clone(),
            result: engine.receive_file(&item),
        }],
        Command::AckTransfer { transfer_id } => vec![Event::TransferAcked {
            transfer_id: transfer_id.clone(),
            result: engine.ack_transfer(&transfer_id),
        }],
    }
}

fn session_from_auth(auth: AuthResponse) -> Session {
    Session {
        access_token: auth.access_token,
        refresh_token: auth.refresh_token,
        expires_at: unix_seconds().saturating_add(auth.expires_in.max(60)),
        user_id: auth.user.id,
        email: auth.user.email,
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn normalized_project_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

fn is_privileged_api_key(value: &str) -> bool {
    let key = value.trim();
    if key.starts_with("sb_secret_") {
        return true;
    }
    let Some(payload) = key.split('.').nth(1) else {
        return false;
    };
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload));
    let Ok(decoded) = decoded else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&decoded)
        .ok()
        .is_some_and(|value| {
            value.get("role").and_then(serde_json::Value::as_str) == Some("service_role")
        })
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

fn response_json<T: for<'de> Deserialize<'de>>(
    response: Response,
    action: &str,
) -> Result<T, String> {
    if !response.status().is_success() {
        return Err(response_error(response, action));
    }
    response
        .json::<T>()
        .map_err(|error| format!("{action} returned invalid JSON: {error}"))
}

fn expect_success(response: Response, action: &str) -> Result<(), String> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(response_error(response, action))
    }
}

fn response_error(response: Response, action: &str) -> String {
    let status = response.status();
    let body = response.text().unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("msg"))
                .or_else(|| value.get("error_description"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    if detail.trim().is_empty() {
        format!("{action} failed with HTTP {status}")
    } else {
        format!("{action} failed with HTTP {status}: {detail}")
    }
}

fn encode_object_path(path: &str) -> String {
    path.split('/')
        .map(|component| {
            percent_encoding::utf8_percent_encode(component, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn receive_directory() -> Result<PathBuf, String> {
    let home = crate::app::home_dir().ok_or_else(|| "Home directory is unavailable".to_owned())?;
    #[cfg(target_os = "android")]
    let path = home.join("Download").join("FastExplorer Inbox");
    #[cfg(not(target_os = "android"))]
    let path = home.join("Downloads").join("FastExplorer Inbox");
    Ok(path)
}

fn safe_received_name(file_name: &str, transfer_id: &str) -> String {
    Path::new(file_name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .map(str::to_owned)
        .unwrap_or_else(|| format!("received-{transfer_id}"))
}

fn unique_destination(dir: &Path, name: &str) -> PathBuf {
    let original = dir.join(name);
    if !original.exists() {
        return original;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("received");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2..10_000 {
        let candidate = match extension {
            Some(extension) => dir.join(format!("{stem} ({index}).{extension}")),
            None => dir.join(format!("{stem} ({index})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{}-{name}", uuid::Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_path_encodes_each_component_without_losing_slashes() {
        assert_eq!(
            encode_object_path("user id/device/file"),
            "user%20id/device/file"
        );
    }

    #[test]
    fn received_name_rejects_path_traversal() {
        assert_eq!(safe_received_name("../../secret.txt", "id"), "secret.txt");
        assert_eq!(safe_received_name("..", "abc"), "received-abc");
    }

    #[test]
    fn blocking_client_lifecycle_stays_off_async_runtime_threads() {
        let runtime = xilem::tokio::runtime::Runtime::new().expect("test Tokio runtime");
        runtime.block_on(async {
            let (request_tx, request_rx) = std::sync::mpsc::channel();
            let (result_tx, mut result_rx) = xilem::tokio::sync::mpsc::unbounded_channel();
            let worker = std::thread::spawn(move || run_blocking_engine(request_rx, result_tx));

            request_tx.send(WorkerRequest::PollInbox).unwrap();
            let result = result_rx.recv().await.expect("blocking worker response");
            assert!(result.poll_completed);
            assert!(result.events.is_empty());
            drop(request_tx);
            worker.join().expect("blocking worker joins cleanly");
        });
    }

    #[test]
    fn privileged_api_keys_are_rejected() {
        assert!(is_privileged_api_key("sb_secret_example"));
        assert!(is_privileged_api_key("x.eyJyb2xlIjoic2VydmljZV9yb2xlIn0.x"));
        assert!(!is_privileged_api_key("sb_publishable_example"));
        assert!(!is_privileged_api_key("x.eyJyb2xlIjoiYW5vbiJ9.x"));

        let mut engine = Engine::default();
        engine.settings.project_url = "https://example.supabase.co".to_owned();
        engine.settings.publishable_key = "sb_secret_example".to_owned();
        assert!(engine.ensure_configured().is_err());
    }

    #[test]
    fn project_url_requires_https_except_real_loopback_hosts() {
        let mut engine = Engine::default();
        engine.settings.publishable_key = "test-key".to_owned();

        engine.settings.project_url = "https://example.supabase.co".to_owned();
        assert!(engine.ensure_configured().is_ok());

        engine.settings.project_url = "http://localhost:54321".to_owned();
        assert!(engine.ensure_configured().is_ok());

        engine.settings.project_url = "http://127.0.0.1:54321".to_owned();
        assert!(engine.ensure_configured().is_ok());

        engine.settings.project_url = "http://localhost.example.com".to_owned();
        assert!(engine.ensure_configured().is_err());
    }
}
