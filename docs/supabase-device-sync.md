# Supabase device sync

FastExplorer can use a Supabase project for account-scoped device registration, file transfer, and text clipboard transfer.

## Project setup

1. Apply `supabase/migrations/20260816_fast_explorer_sync.sql` to the project.
2. Keep the generated `fast-explorer-transfers` Storage bucket private.
3. In Auth email templates, make the Magic Link template include `{{ .Token }}` when using the 6-digit OTP flow.
4. In FastExplorer, open **Settings → Device sync (Supabase)** and enter the project URL plus a publishable/anon key.
5. Set a device name, apply the sync settings, then request and verify the email OTP.

Never put a secret or `service_role` key in the application. FastExplorer rejects `sb_secret_...` and legacy JWTs whose role is `service_role`; use a publishable/anon key. FastExplorer accesses exposed tables and Storage with the signed-in user's JWT and the migration's RLS policies.

## Current transfer behavior

- Selected local files can be sent to another registered device. TailDrive/archive virtual files must first be copied locally.
- Text from the OS clipboard can be sent to another registered device. Received clipboard text is not applied automatically; the receiver explicitly presses **Copy**.
- Incoming items are polled while the FastExplorer process is alive. Android posts a local notification and tapping it opens Settings/device sync.
- This is not an FCM push channel: a fully terminated Android process cannot receive a new-transfer notification until FastExplorer runs again. Adding app-killed push delivery requires a separate push provider/credential path.

## Android killed-state push

Android push uses FCM; Supabase remains the server-side source of truth and sender.
The normal path is transfer INSERT -> Supabase Database Webhook -> `fast-explorer-push` Edge Function -> FCM.
FCM notification messages can appear from the Android system even when the FastExplorer process is not running.
Android's explicit **Force stop** is different: the OS/FCM can suppress delivery until the user launches the app again.

1. Register Android package `dev.oligami.fastexplorer` in a Firebase project.
2. Put its `google-services.json` at `android-gradle/app/google-services.json` before building.
3. Apply `supabase/migrations/20260817_fast_explorer_push.sql`.
4. Create a Firebase service account allowed to send FCM HTTP v1 messages.
5. Store the complete service-account JSON in the Supabase Edge Function secret `FIREBASE_SERVICE_ACCOUNT_JSON`.
6. Create a random secret as `FASTEXPLORER_PUSH_WEBHOOK_SECRET`.
7. Deploy with `supabase functions deploy fast-explorer-push --no-verify-jwt`.
8. In Supabase **Database → Webhooks**, create an `INSERT` webhook for `public.fast_explorer_transfers`.
9. Target `https://<project-ref>.supabase.co/functions/v1/fast-explorer-push` with method POST.
10. Add header `x-fast-explorer-webhook-secret` with exactly the same random secret.

The Edge Function looks up only the receiver's stored FCM token using the server-side service-role credential. Clipboard contents and file payloads are never put in the push notification. If FCM reports an unregistered token, the function clears it so subsequent sends do not repeatedly target a dead installation.

The existing five-second inbox poll remains as a delivery-recovery path while FastExplorer is running. It is no longer required to keep the process alive merely to get the initial Android notification once FCM is configured.
