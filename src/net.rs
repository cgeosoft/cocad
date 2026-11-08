//! HTTP client setup, shared by all network code.

use std::time::Duration;

/// An agent that stops each request after `timeout`.
pub fn agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder().timeout_global(Some(timeout));
    // Linux uses rustls, so the app needs no OpenSSL. `cargo check` for Windows
    // and macOS must work on Linux, and ring needs a C toolchain for the target.
    #[cfg(not(target_os = "linux"))]
    let config = config.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .build(),
    );
    config.build().into()
}
