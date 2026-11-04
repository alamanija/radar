//! Live Clerk sign-in check: runs the browser PKCE flow and prints the
//! decoded (unverified) JWT header + claims of the access token and id_token.
//! Raw tokens are never printed.
//!
//! ```sh
//! RADAR_CLERK_FRONTEND_API=https://<instance>.clerk.accounts.dev \
//! RADAR_CLERK_CLIENT_ID=<oauth client id> \
//! cargo run -p radar-core --example sign_in
//! ```

use radar_core::auth::{self, AuthConfig};
use std::process::Command;
use std::time::Duration;

fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        // Quote the URL so cmd doesn't treat `&` in the query as a separator.
        c.arg("/C").raw_arg(format!("start \"\" \"{url}\""));
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().map(|_| ())
}

fn print_claims(label: &str, jwt: Option<&str>) {
    let Some(jwt) = jwt else {
        println!("{label}: (absent)");
        return;
    };
    match auth::decode_jwt_unverified(jwt) {
        Some((header, claims)) => {
            let pretty =
                |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_default();
            println!("{label} header:\n{}", pretty(&header));
            println!("{label} claims:\n{}", pretty(&claims));
        }
        None => println!("{label}: not a JWT (opaque, {} chars)", jwt.len()),
    }
}

fn main() {
    let Some(cfg) = AuthConfig::from_env() else {
        eprintln!("set RADAR_CLERK_FRONTEND_API and RADAR_CLERK_CLIENT_ID");
        std::process::exit(2);
    };
    let flow = auth::sign_in(
        &cfg,
        |url| {
            println!("Opening the browser to sign in…");
            if let Err(e) = open_browser(url) {
                eprintln!("could not open a browser ({e}); visit:\n{url}");
            }
        },
        Duration::from_secs(300),
    );
    match radar_core::runtime().block_on(flow) {
        Ok((tokens, account)) => {
            println!("account: {account:?}");
            println!(
                "expires_at: {}  refresh_token: {}",
                tokens.expires_at,
                if tokens.refresh_token.is_some() {
                    "present"
                } else {
                    "absent"
                }
            );
            print_claims("access_token", Some(&tokens.access_token));
            print_claims("id_token", tokens.id_token.as_deref());
        }
        Err(e) => {
            eprintln!("sign-in failed: {e}");
            std::process::exit(1);
        }
    }
}
