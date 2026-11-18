//! Decides who may talk to the service.
//!
//! The service starts programs with full permissions on this machine, so who
//! reaches it matters more than anything else it does. It listens on
//! `127.0.0.1` only, which keeps other machines out, but that still leaves
//! two kinds of strangers:
//!
//! - **Web pages.** Any page open in a browser may open a WebSocket to
//!   `127.0.0.1`; browsers do not apply their usual cross-site rules to
//!   WebSockets. The `Origin` header names the page, so only the desktop
//!   app's own origin and origins listed in the configuration are let in.
//!   A page can also point a name it owns at `127.0.0.1` to look local
//!   (DNS rebinding), which the `Host` header gives away.
//! - **Other people's programs on the same machine.** They pass both checks
//!   above. The token stops them: it is written to `service.json`, which only
//!   the person running the service can read.
//!
//! A refusal never says which check failed, so a stranger learns nothing
//! about how close they came.

use subtle::ConstantTimeEq;
use thiserror::Error;

/// The origins the desktop app's window has, depending on the platform.
///
/// Linux and macOS use Tauri's own scheme; Windows serves the app over
/// `http` by default and over `https` when told to.
const DESKTOP_ORIGINS: [&str; 3] = [
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
];

/// A connection the service will not talk to.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("Refused.")]
pub struct Refused;

/// Checks the headers of a request to connect, before anything else happens.
///
/// `host` and `origin` are the headers as sent, if sent; `port` is the port the
/// service listens on; `allowed_origins` comes from the configuration.
pub fn admit(
    host: Option<&str>,
    origin: Option<&str>,
    port: u16,
    allowed_origins: &[String],
) -> Result<(), Refused> {
    let host = host.ok_or(Refused)?;
    let local = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if !local.iter().any(|name| name.eq_ignore_ascii_case(host)) {
        return Err(Refused);
    }

    match origin {
        // Programs such as the terminal app send no origin. A browser always does.
        None => Ok(()),
        Some(origin) if DESKTOP_ORIGINS.contains(&origin) => Ok(()),
        Some(origin) if allowed_origins.iter().any(|allowed| allowed == origin) => Ok(()),
        Some(_) => Err(Refused),
    }
}

/// Whether a token sent by a client is the service's token.
///
/// Takes as long for a token that is nearly right as for one that is all
/// wrong, so the token cannot be guessed one character at a time.
pub fn token_matches(expected: &str, given: &str) -> bool {
    expected.as_bytes().ct_eq(given.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    const PORT: u16 = 7317;

    fn development() -> Vec<String> {
        vec!["http://localhost:4200".to_string()]
    }

    lets_expect! {
        expect(admit(host, origin, PORT, &allowed)) {
            let allowed = Vec::new();
            let origin = None;

            when(host = Some("127.0.0.1:7317")) as the_address_and_port {
                when(origin = None) as a_program_without_an_origin to be_admitted { be_ok }
                when(origin = Some("tauri://localhost")) as the_desktop_app_on_linux_or_macos to be_admitted { be_ok }
                when(origin = Some("http://tauri.localhost")) as the_desktop_app_on_windows to be_admitted { be_ok }
                when(origin = Some("https://tauri.localhost")) as the_desktop_app_on_windows_over_https to be_admitted { be_ok }
                when(origin = Some("https://example.com")) as any_web_page to be_refused { be_err }
                when(origin = Some("null")) as a_page_hiding_its_origin to be_refused { be_err }
                when(origin = Some("")) as an_empty_origin to be_refused { be_err }
                when(origin = Some("http://localhost:4200")) as the_development_app_not_listed to be_refused { be_err }
                when(origin = Some("tauri://localhost.example.com")) as a_look_alike_of_the_desktop_app to be_refused { be_err }
                when(origin = Some("TAURI://LOCALHOST")) as the_desktop_origin_in_capitals to be_refused { be_err }

                when the_development_app_is_listed {
                    let allowed = development();
                    when(origin = Some("http://localhost:4200")) as its_origin to be_admitted { be_ok }
                    when(origin = Some("http://localhost:4201")) as another_port to be_refused { be_err }
                    when(origin = Some("http://localhost:4200/")) as its_origin_with_a_slash to be_refused { be_err }
                    when(origin = Some("https://example.com")) as any_other_web_page to be_refused { be_err }
                }
            }

            when(host = Some("localhost:7317")) as the_name_and_port {
                when(origin = None) as a_program to be_admitted { be_ok }
                when(origin = Some("https://example.com")) as a_web_page to be_refused { be_err }
            }

            when(host = Some("LOCALHOST:7317")) as the_name_in_capitals to be_admitted { be_ok }
            when(host = None) as no_host_header to be_refused { be_err }
            when(host = Some("127.0.0.1")) as the_address_without_the_port to be_refused { be_err }
            when(host = Some("127.0.0.1:7318")) as another_port to be_refused { be_err }
            when(host = Some("evil.example.com:7317")) as a_web_page_pointing_its_name_here to be_refused { be_err }
            when(host = Some("localhost.example.com:7317")) as a_look_alike_name to be_refused { be_err }
            when(host = Some("192.168.1.10:7317")) as a_network_address to be_refused { be_err }
            when(host = Some("[::1]:7317")) as the_ipv6_loopback_the_service_does_not_listen_on to be_refused { be_err }
            when(host = Some("")) as an_empty_host to be_refused { be_err }

            when a_web_page_pointing_its_name_here_with_an_allowed_origin {
                let host = Some("evil.example.com:7317");
                let origin = Some("http://localhost:4200");
                let allowed = development();
                to be_refused { be_err }
            }
        }

        expect(token_matches("a1b2c3", given)) {
            when(given = "a1b2c3") as the_token to match_it { be_true }
            when(given = "a1b2c4") as a_token_wrong_in_its_last_character to be_refused { be_false }
            when(given = "a1b2c") as a_shorter_token to be_refused { be_false }
            when(given = "a1b2c3d") as a_longer_token to be_refused { be_false }
            when(given = "") as an_empty_token to be_refused { be_false }
            when(given = "A1B2C3") as the_token_in_capitals to be_refused { be_false }
        }
    }
}
