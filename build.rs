use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=tailscale-bridge");
    println!("cargo:rerun-if-changed=tailscale-bridge/go.mod");
    println!("cargo:rerun-if-changed=tailscale-bridge/go.sum");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
    println!("cargo:rerun-if-env-changed=ANDROID_HOME");
    println!("cargo:rerun-if-env-changed=FASTEXPLORER_TSNET_ARTIFACT");
    println!("cargo:rustc-check-cfg=cfg(fastexplorer_tsnet)");
    println!("cargo:rustc-check-cfg=cfg(fastexplorer_tsnet_dynamic)");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "linux" => build_linux(),
        "android" => build_android(),
        "windows" => build_windows(),
        _ => println!("cargo:warning=embedded Tailscale is unavailable on {target_os}"),
    }
}

fn bridge_dir() -> PathBuf {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir")).join("tailscale-bridge")
}

fn out_dir() -> PathBuf {
    PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"))
}

fn go_command() -> Command {
    let mut command = Command::new("go");
    command.env("GOTOOLCHAIN", "go1.26.5");
    command
}

fn run(mut command: Command, description: &str) {
    let status = command.status().unwrap_or_else(|error| {
        panic!("failed to run {description}: {error}");
    });
    assert!(status.success(), "{description} failed with {status}");
}

fn build_linux() {
    let output = out_dir().join("libfastexplorer_tsnet.a");
    let mut command = go_command();
    command
        .current_dir(bridge_dir())
        .env("CGO_ENABLED", "1")
        .arg("build")
        .arg("-buildmode=c-archive")
        .arg("-o")
        .arg(&output)
        .arg(".");
    run(command, "Linux embedded Tailscale bridge build");

    println!("cargo:rustc-cfg=fastexplorer_tsnet");
    println!("cargo:rustc-link-search=native={}", out_dir().display());
    println!("cargo:rustc-link-lib=static=fastexplorer_tsnet");
    for library in ["pthread", "dl", "m", "resolv"] {
        println!("cargo:rustc-link-lib={library}");
    }
}

fn android_tailscale_modfile() -> PathBuf {
    const TAILSCALE_VERSION: &str = "v1.98.8";
    let output = go_command()
        .args(["env", "GOMODCACHE"])
        .output()
        .expect("query Go module cache");
    assert!(output.status.success(), "go env GOMODCACHE failed");
    let module_cache = String::from_utf8(output.stdout)
        .expect("GOMODCACHE is not UTF-8")
        .trim()
        .to_owned();
    let original = PathBuf::from(module_cache).join(format!("tailscale.com@{TAILSCALE_VERSION}"));
    let patched = out_dir().join("tailscale-fastexplorer");
    if !patched.is_dir() {
        let mut copy = Command::new("cp");
        copy.arg("-R").arg(&original).arg(&patched);
        run(copy, "copy Tailscale source for Android patch");
    }

    let logpolicy = patched.join("logpolicy/logpolicy.go");
    let mut source = fs::read_to_string(original.join("logpolicy/logpolicy.go"))
        .expect("read Tailscale logpolicy");
    let declaration = "var getLogTargetOnce struct {";
    assert!(
        source.contains(declaration),
        "unexpected Tailscale logpolicy layout"
    );
    source = source.replacen(
        declaration,
        "var fastExplorerAndroidLogsDir string\n\n// SetFastExplorerAndroidLogsDir configures app-private log storage for tsnet on Android.\nfunc SetFastExplorerAndroidLogsDir(dir string) { fastExplorerAndroidLogsDir = dir }\n\nvar getLogTargetOnce struct {",
        1,
    );
    let logs_dir = "func LogsDir(logf logger.Logf) string {\n\tif d := os.Getenv(\"TS_LOGS_DIR\"); d != \"\" {";
    assert!(
        source.contains(logs_dir),
        "unexpected Tailscale LogsDir implementation"
    );
    source = source.replacen(
        logs_dir,
        "func LogsDir(logf logger.Logf) string {\n\tif runtime.GOOS == \"android\" && fastExplorerAndroidLogsDir != \"\" {\n\t\tif fi, err := os.Stat(fastExplorerAndroidLogsDir); err == nil && fi.IsDir() {\n\t\t\treturn fastExplorerAndroidLogsDir\n\t\t}\n\t}\n\tif d := os.Getenv(\"TS_LOGS_DIR\"); d != \"\" {",
        1,
    );
    let mut permissions = fs::metadata(&logpolicy)
        .expect("stat patched Tailscale logpolicy")
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o644);
    }
    #[cfg(windows)]
    permissions.set_readonly(false);
    fs::set_permissions(&logpolicy, permissions).expect("make patched logpolicy writable");
    fs::write(&logpolicy, source).expect("write patched Tailscale logpolicy");

    let bridge_mod = fs::read_to_string(bridge_dir().join("go.mod")).expect("read bridge go.mod");
    let modfile = out_dir().join("fast-explorer-android.mod");
    fs::write(
        &modfile,
        format!(
            "{bridge_mod}\nreplace tailscale.com => {}\n",
            patched.display()
        ),
    )
    .expect("write Android bridge modfile");
    fs::copy(
        bridge_dir().join("go.sum"),
        out_dir().join("fast-explorer-android.sum"),
    )
    .expect("copy Android bridge go.sum");
    modfile
}

fn build_android() {
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let (go_arch, clang_name) = match target_arch.as_str() {
        "aarch64" => ("arm64", "aarch64-linux-android30-clang"),
        "x86_64" => ("amd64", "x86_64-linux-android30-clang"),
        other => panic!("unsupported FastExplorer Android architecture: {other}"),
    };
    let ndk = android_ndk();
    let clang = ndk
        .join("toolchains/llvm/prebuilt/linux-x86_64/bin")
        .join(clang_name);
    assert!(
        clang.is_file(),
        "Android NDK compiler not found: {}",
        clang.display()
    );

    let output = out_dir().join("libfastexplorer_tsnet.so");
    let modfile = android_tailscale_modfile();
    let mut command = go_command();
    command
        .current_dir(bridge_dir())
        .env("CGO_ENABLED", "1")
        .env("GOOS", "android")
        .env("GOARCH", go_arch)
        .env("CC", &clang)
        .arg("build")
        .arg("-modfile")
        .arg(&modfile)
        .arg("-buildmode=c-shared")
        .arg("-ldflags")
        .arg("-extldflags=-Wl,-z,max-page-size=16384")
        .arg("-o")
        .arg(&output)
        .arg(".");
    run(command, "Android embedded Tailscale bridge build");

    if let Some(artifact) = env::var_os("FASTEXPLORER_TSNET_ARTIFACT") {
        let artifact = PathBuf::from(artifact);
        if let Some(parent) = artifact.parent() {
            fs::create_dir_all(parent).expect("create Android Tailscale artifact directory");
        }
        fs::copy(&output, &artifact).expect("copy deterministic Android Tailscale artifact");
    }

    println!("cargo:rustc-cfg=fastexplorer_tsnet");
    println!("cargo:rustc-link-search=native={}", out_dir().display());
    println!("cargo:rustc-link-lib=dylib=fastexplorer_tsnet");
    println!(
        "cargo:rustc-env=FASTEXPLORER_TSNET_ANDROID_SO={}",
        output.display()
    );
}

fn build_windows() {
    println!("cargo:rustc-cfg=fastexplorer_tsnet_dynamic");
    let host = env::var("HOST").unwrap_or_default();
    if !host.contains("windows") {
        println!(
            "cargo:warning=Windows embedded Tailscale DLL build skipped while cross-checking from {host}"
        );
        return;
    }

    let cargo_out = out_dir();
    let output_dir = cargo_out
        .ancestors()
        .nth(3)
        .expect("Cargo profile output directory");
    let output = output_dir.join("fast_explorer_tsnet.dll");
    let mut command = go_command();
    command
        .current_dir(bridge_dir())
        .env("CGO_ENABLED", "1")
        .env("GOOS", "windows")
        .env("GOARCH", "amd64")
        .arg("build")
        .arg("-buildmode=c-shared")
        .arg("-o")
        .arg(&output)
        .arg(".");
    run(command, "Windows embedded Tailscale DLL build");
    println!("cargo:warning=embedded Tailscale DLL: {}", output.display());
}

fn android_ndk() -> PathBuf {
    if let Some(path) = env::var_os("ANDROID_NDK_HOME") {
        return PathBuf::from(path);
    }
    if let Some(sdk) = env::var_os("ANDROID_HOME") {
        let candidate = PathBuf::from(sdk).join("ndk/27.3.13750724");
        if candidate.is_dir() {
            return candidate;
        }
    }
    let home = env::var_os("HOME").expect("HOME is required to locate the Android NDK");
    Path::new(&home).join("Android/Sdk/ndk/27.3.13750724")
}
