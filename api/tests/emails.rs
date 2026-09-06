//! Request specs for the branded transactional emails.
//!
//! The bodies come from `emails/*.html`, which Erno reads off disk and fills in
//! by string substitution. Nothing type-checks a placeholder: a renamed or
//! misspelled `{{key}}` ships silently as literal braces in a real user's
//! inbox, and the config doc comment advertises a `{{app_name}}` that no job
//! actually supplies. These specs are the only thing standing between that and
//! production.

mod common;

use erno::jobs::send_password_reset_email_job::{
    SendPasswordResetEmailArgs, SendPasswordResetEmailJob,
};
use erno::jobs::send_verification_email_job::{
    SendVerificationEmailArgs, SendVerificationEmailJob,
};
use erno::tests::verified_user;

/// Every placeholder Erno leaves unsubstituted survives as literal `{{...}}`.
fn assert_fully_rendered(body: &str) {
    assert!(
        !body.contains("{{"),
        "template left an unsubstituted placeholder: {body}"
    );
}

/// Distinguishes the branded body from Erno's plain fallback, which is a bare
/// pair of `<p>` tags with no table markup and no wordmark.
fn assert_branded(body: &str) {
    assert!(body.contains("TERYON"), "body is missing the wordmark");
    assert!(
        body.contains("role=\"presentation\""),
        "body is not the table-based branded layout"
    );
}

#[tokio::test]
async fn verification_email_is_branded_and_fully_rendered() {
    let t = common::setup().await;
    let user = verified_user(&t.db, "alex@acme.com", "hunter22password").await;

    t.execute_job::<SendVerificationEmailJob>(SendVerificationEmailArgs {
        user_id: user.id,
        email: user.email.clone(),
        raw_token: "verification-token".to_string(),
    })
    .await
    .expect("verification job succeeds");

    let sent = t.sent_emails();
    assert_eq!(sent.len(), 1, "exactly one email is sent");
    let body = sent[0].body_html.as_deref().expect("an HTML body");

    assert_branded(body);
    assert_fully_rendered(body);
    assert!(body.contains("Confirm your email address"));
    assert!(body.contains("alex@acme.com"), "{{email}} is filled in");
    assert!(
        body.contains("/verify-email?token=verification-token"),
        "{{verify_url}} is filled in"
    );
    assert!(
        body.contains("Expires in 24 hours"),
        "{{expiry_hours}} is filled in from auth.one_time_token_expiry_hours"
    );
}

#[tokio::test]
async fn password_reset_email_is_branded_and_fully_rendered() {
    let t = common::setup().await;
    let user = verified_user(&t.db, "alex@acme.com", "hunter22password").await;

    t.execute_job::<SendPasswordResetEmailJob>(SendPasswordResetEmailArgs {
        user_id: user.id,
        email: user.email.clone(),
        raw_token: "reset-token".to_string(),
    })
    .await
    .expect("password reset job succeeds");

    let sent = t.sent_emails();
    assert_eq!(sent.len(), 1, "exactly one email is sent");
    let body = sent[0].body_html.as_deref().expect("an HTML body");

    assert_branded(body);
    assert_fully_rendered(body);
    assert!(body.contains("Set a new password"));
    assert!(body.contains("alex@acme.com"), "{{email}} is filled in");
    assert!(
        body.contains("/reset-password?token=reset-token"),
        "{{reset_url}} is filled in"
    );
    assert!(body.contains("Expires in 24 hours"));
}

/// The templates are read relative to the process working directory. Cargo runs
/// tests from the package root, which is also where the binary is started, so a
/// missing file here means the deployed path is wrong too.
#[tokio::test]
async fn the_configured_template_directory_holds_both_files() {
    let t = common::setup().await;
    let dir = t
        .config
        .email_templates_dir
        .as_deref()
        .expect("email_templates_dir is configured");

    for name in ["verification.html", "password_reset.html"] {
        let path = std::path::Path::new(dir).join(name);
        assert!(path.exists(), "{} is missing", path.display());
    }
}
