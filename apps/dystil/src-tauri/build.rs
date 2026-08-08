fn resolve_build_channel() -> String {
    let explicit = std::env::var("DYSTIL_BUILD_CHANNEL")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    match explicit.as_deref() {
        Some("local" | "beta" | "prod") => explicit.unwrap(),
        Some(other) => {
            panic!("invalid DYSTIL_BUILD_CHANNEL={other}; expected one of local, beta, prod")
        }
        None => {
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                "prod".to_string()
            } else {
                "local".to_string()
            }
        }
    }
}

fn write_generated_app_config() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let channel = resolve_build_channel();
    let config_path = std::path::Path::new(&manifest_dir)
        .join("../config")
        .join(format!("app-config.{channel}.json"));
    let generated_config_path =
        std::path::Path::new(&manifest_dir).join("../lib/generated/app-config.json");

    // Rust-only entrypoint guard:
    // cargo/tauri can compile without the JS pipeline, so rebuild the same
    // generated artifact here before Rust code includes it.
    println!("cargo:rerun-if-env-changed=DYSTIL_BUILD_CHANNEL");
    println!("cargo:rustc-env=DYSTIL_BUILD_CHANNEL={channel}");
    println!("cargo:rerun-if-changed={}", config_path.display());

    let raw = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        panic!("failed to read {}: {}", config_path.display(), e);
    });
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        panic!("failed to parse {}: {}", config_path.display(), e);
    });
    if !parsed.is_object() {
        panic!("{} must contain a JSON object", config_path.display());
    }

    if let Some(parent) = generated_config_path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            panic!("failed to create {}: {}", parent.display(), e);
        });
    }
    std::fs::write(&generated_config_path, "{}\n").unwrap_or_else(|e| {
        panic!(
            "failed to write generated config {}: {}",
            generated_config_path.display(),
            e
        )
    });
    println!("cargo:rerun-if-changed={}", generated_config_path.display());
}

fn configure_cloud_build() {
    let cloud_sync = std::env::var_os("CARGO_FEATURE_CLOUD_SYNC").is_some();
    let configured_url = std::env::var("DYSTIL_CLOUD_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());

    println!("cargo:rerun-if-env-changed=DYSTIL_CLOUD_BASE_URL");
    println!("cargo:rerun-if-env-changed=DYSTIL_TELEMETRY_ENDPOINT");

    match (cloud_sync, configured_url) {
        (false, None) => {}
        (false, Some(_)) => panic!(
            "DYSTIL_CLOUD_BASE_URL is set but the cloud-sync feature is disabled; \
             refuse to embed a cloud endpoint in a community build"
        ),
        (true, None) => {
            panic!("cloud-sync requires DYSTIL_CLOUD_BASE_URL to be a non-empty HTTPS URL")
        }
        (true, Some(url)) => {
            let parsed = url::Url::parse(&url).unwrap_or_else(|error| {
                panic!("DYSTIL_CLOUD_BASE_URL must be a valid URL: {error}")
            });
            let localhost_http = parsed.scheme() == "http"
                && matches!(
                    parsed.host_str(),
                    Some("localhost") | Some("127.0.0.1") | Some("::1")
                );
            let release = std::env::var("PROFILE").as_deref() == Ok("release");
            if parsed.host_str().is_none()
                || (parsed.scheme() != "https" && (release || !localhost_http))
            {
                panic!("DYSTIL_CLOUD_BASE_URL must be HTTPS (debug builds may use localhost HTTP)");
            }
            println!("cargo:rustc-env=DYSTIL_CLOUD_BASE_URL={url}");
        }
    }

    let telemetry_endpoint = std::env::var("DYSTIL_TELEMETRY_ENDPOINT")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());
    match (cloud_sync, telemetry_endpoint) {
        (false, None) => {}
        (false, Some(_)) => panic!(
            "DYSTIL_TELEMETRY_ENDPOINT is set but the cloud-sync feature is disabled; \
             refuse to embed a telemetry endpoint in a community build"
        ),
        (true, None) => {}
        (true, Some(endpoint)) => {
            let parsed = url::Url::parse(&endpoint).unwrap_or_else(|error| {
                panic!("DYSTIL_TELEMETRY_ENDPOINT must be a valid URL: {error}")
            });
            let localhost_http = parsed.scheme() == "http"
                && matches!(
                    parsed.host_str(),
                    Some("localhost") | Some("127.0.0.1") | Some("::1")
                );
            let release = std::env::var("PROFILE").as_deref() == Ok("release");
            if parsed.host_str().is_none()
                || (parsed.scheme() != "https" && (release || !localhost_http))
            {
                panic!(
                    "DYSTIL_TELEMETRY_ENDPOINT must be HTTPS (debug builds may use localhost HTTP)"
                );
            }
            println!("cargo:rustc-env=DYSTIL_TELEMETRY_ENDPOINT={endpoint}");
        }
    }
}

/// Compile notification_panel.swift into a static library for native macOS notifications.
#[cfg(target_os = "macos")]
fn build_notification_panel() {
    use std::path::PathBuf;
    use std::process::Command;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let swift_src = PathBuf::from("swift/notification_panel.swift");
    let lib_path = out_dir.join("libnotification_panel.a");

    println!("cargo:rerun-if-changed=swift/notification_panel.swift");

    if !swift_src.exists() {
        println!("cargo:warning=swift/notification_panel.swift not found, skipping native notification panel");
        build_notification_panel_stub(&out_dir, &lib_path);
        return;
    }

    let sdk_path = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let sdk_path = sdk_path.trim().to_string();

    let target_arch =
        std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".to_string());
    let swift_target = if target_arch == "x86_64" {
        "x86_64-apple-macos13.0"
    } else {
        "arm64-apple-macos13.0"
    };

    let output = Command::new("swiftc")
        .args([
            "-emit-library",
            "-static",
            "-module-name",
            "NotificationPanel",
            "-swift-version",
            "5",
            "-sdk",
            &sdk_path,
            "-target",
            swift_target,
            "-O",
            "-whole-module-optimization",
            "-o",
        ])
        .arg(&lib_path)
        .arg(&swift_src)
        .output()
        .expect("failed to run swiftc for notification_panel");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!(
            "cargo:warning=swiftc failed for notification_panel.swift: {}",
            stderr.chars().take(500).collect::<String>()
        );
        build_notification_panel_stub(&out_dir, &lib_path);
        return;
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=notification_panel");
    // SwiftUI needs AppKit (already linked) and SwiftUI framework
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,SwiftUI");
}

/// Build a C stub when SwiftUI notification panel is not available.
#[cfg(target_os = "macos")]
fn build_notification_panel_stub(out_dir: &std::path::Path, lib_path: &std::path::Path) {
    use std::process::Command;

    let stub_src = out_dir.join("notification_panel_stub.c");
    std::fs::write(
        &stub_src,
        r#"// Stub: SwiftUI notification panel not available
#include <stdlib.h>

typedef void (*action_callback_t)(const char*);

void notif_set_action_callback(action_callback_t cb) { (void)cb; }
int notif_show(const char* json) { (void)json; return -2; }
int notif_hide(void) { return -2; }
int notif_is_available(void) { return 0; }
void notif_free_string(char* ptr) { if (ptr) free(ptr); }
"#,
    )
    .expect("failed to write notification panel stub");

    let target_arch =
        std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".to_string());
    let cc_arch = if target_arch == "x86_64" {
        "x86_64"
    } else {
        "arm64"
    };
    let status = Command::new("cc")
        .args(["-c", "-arch", cc_arch, "-o"])
        .arg(out_dir.join("notification_panel_stub.o").to_str().unwrap())
        .arg(stub_src.to_str().unwrap())
        .status()
        .expect("failed to compile notification panel stub");
    assert!(
        status.success(),
        "notification panel stub compilation failed"
    );

    let status = Command::new("ar")
        .args(["rcs"])
        .arg(lib_path)
        .arg(out_dir.join("notification_panel_stub.o").to_str().unwrap())
        .status()
        .expect("failed to create notification panel stub archive");
    assert!(status.success(), "notification panel stub archive failed");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=notification_panel");
}

fn main() {
    write_generated_app_config();
    configure_cloud_build();
    tauri_helper::generate_command_file(tauri_helper::TauriHelperOptions::default());

    // Workaround: tauri-helper's macro looks for files starting with "{crate_name}_"
    // but generate_command_file writes "{crate_name}.txt" (no underscore before extension).
    // Copy the file so the prefix check matches.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let commands_dir = std::path::PathBuf::from(&manifest_dir)
        .join("target")
        .join("tauri_commands_list");
    let src = commands_dir.join("dystil_app.txt");
    let dst = commands_dir.join("dystil_app_commands.txt");
    if src.exists() {
        let _ = std::fs::copy(&src, &dst);
    }

    // Stamp the build time so `main.rs` can self-quiesce Sentry reports
    // for ancient builds. This makes the Sentry inbox reflect what's
    // actually running today; users who never update gradually fall
    // silent instead of polluting signal for months after a known bug
    // has been fixed. 90-day TTL is enforced in the `before_send` hook.
    let build_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=DYSTIL_BUILD_UNIX_TIME={}", build_time);
    // Re-run the build script on every compile so the timestamp is fresh.
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "macos")]
    {
        // Swift runtime rpaths used by the native notification panel.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

        if let Ok(output) = std::process::Command::new("xcode-select")
            .arg("-p")
            .output()
        {
            let xcode_dev = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let toolchain_swift = format!(
                "{}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx",
                xcode_dev
            );
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", toolchain_swift);
        }

        // Build SwiftUI notification panel
        build_notification_panel();

        // Stage permission-flow's resource bundle for Tauri to pick up.
        copy_permission_flow_bundle();
    }

    // Empty stub on non-macOS so the resource entry in every tauri*.conf.json
    // resolves to something. The bundle is macOS-only at runtime; this just
    // keeps the bundler glob from erroring on Linux/Windows builds.
    #[cfg(not(target_os = "macos"))]
    {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let stub =
            std::path::PathBuf::from(&manifest_dir).join("PermissionFlow_PermissionFlow.bundle");
        if !stub.exists() {
            std::fs::create_dir_all(&stub).ok();
            std::fs::write(stub.join(".placeholder"), b"").ok();
        }
    }

    // Windows MSVC: provide the GCC `__builtin_bswap{16,32,64}` intrinsics
    // as real functions. aws-lc-sys (pulled in by rustls 0.23) ships C
    // that calls them, but cl.exe doesn't recognize the names, so it
    // emits them as unresolved externals and the link fails:
    //
    //   libaws_lc_sys-...md4.o : error LNK2001: unresolved external
    //       symbol __builtin_bswap32
    //
    // c/bswap_shim.c provides them as wrappers around MSVC's
    // `_byteswap_*` intrinsics; cl.exe inlines those, so the runtime
    // cost is zero. No-op on non-MSVC targets.
    //
    // Note: `cfg(target_env = "msvc")` in build.rs evaluates against
    // the *build host*, not the build target. For cross-compiles
    // (CI builds for Windows MSVC from a macOS or Linux runner) we
    // have to read CARGO_CFG_TARGET_ENV instead.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rerun-if-changed=c/bswap_shim.c");
        cc::Build::new()
            .file("c/bswap_shim.c")
            .compile("bswap_shim");
    }

    tauri_build::build()
}

/// Stage `PermissionFlow_PermissionFlow.bundle` into `src-tauri/` so Tauri
/// bundles it into `Contents/Resources/`. Missing it crashes onboarding with
/// `fatalError` on the first localized string in a shipped `.app`.
///
/// Source path comes from `DEP_TAURI_PLUGIN_PERMISSION_FLOW_BUNDLE_DIR`,
/// which the plugin's build.rs re-exports from upstream `permission-flow`
/// via Cargo `links` metadata.
#[cfg(target_os = "macos")]
fn copy_permission_flow_bundle() {
    let bundle_name = "PermissionFlow_PermissionFlow.bundle";

    println!("cargo:rerun-if-env-changed=DEP_TAURI_PLUGIN_PERMISSION_FLOW_BUNDLE_DIR");

    let bundle_src = std::env::var("DEP_TAURI_PLUGIN_PERMISSION_FLOW_BUNDLE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| panic!("DEP_TAURI_PLUGIN_PERMISSION_FLOW_BUNDLE_DIR not set"));

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let bundle_dst = std::path::PathBuf::from(&manifest_dir).join(bundle_name);

    // Missing source means swift-rs's SwiftPM build didn't emit the bundle
    // (CI cache layering, scratch-path mismatch, etc.). Release builds must
    // ship the real bundle — hard-fail. Debug builds only need the
    // path to exist so tauri-build's resource validation passes; same
    // empty-stub trick mlx.metallib uses above.
    if !bundle_src.exists() {
        let is_release = std::env::var("PROFILE").as_deref() == Ok("release");
        let msg = format!(
            "{} missing at {}; swift-rs didn't emit it",
            bundle_name,
            bundle_src.display(),
        );
        if is_release {
            panic!("{msg}");
        }
        println!("cargo:warning={msg} (debug build, staging empty stub)");
        if !bundle_dst.exists() {
            let _ = std::fs::create_dir_all(&bundle_dst);
            let _ = std::fs::write(bundle_dst.join(".placeholder"), b"");
        }
        return;
    }

    if bundle_dst.exists() {
        let _ = std::fs::remove_dir_all(&bundle_dst);
    }

    if let Err(e) = copy_dir_all(&bundle_src, &bundle_dst) {
        panic!(
            "copy {} → {}: {e}",
            bundle_src.display(),
            bundle_dst.display()
        );
    }
}

#[cfg(target_os = "macos")]
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}
