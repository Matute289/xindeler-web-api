use crate::config::SmtpConfig;
use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::Duration;

const SMTP_TIMEOUT: Duration = Duration::from_secs(10);

struct Mailer {
    transport: SmtpTransport,
    from: Mailbox,
}

static MAILER: OnceLock<Option<Mailer>> = OnceLock::new();

/// `None` means SMTP isn't configured — matches the Python service's
/// `if not all([SMTP_HOST, SMTP_USER, SMTP_PASS]): return`, so local dev
/// and most tests never need real credentials.
pub fn initialize() -> Result<(), String> {
    let config = crate::config::get();
    let mailer = match &config.smtp {
        Some(smtp) => Some(build_mailer(smtp)?),
        None => None,
    };
    MAILER
        .set(mailer)
        .map_err(|_| "mailer was already initialized".to_string())
}

fn build_mailer(smtp: &SmtpConfig) -> Result<Mailer, String> {
    let transport = SmtpTransport::starttls_relay(&smtp.host)
        .map_err(|_| "SMTP_HOST is invalid")?
        .port(smtp.port)
        .credentials(Credentials::new(
            smtp.user.clone(),
            smtp.password().to_owned(),
        ))
        .timeout(Some(SMTP_TIMEOUT))
        .build();
    let from = smtp.from.parse().map_err(|_| "MAIL_FROM is invalid")?;
    Ok(Mailer { transport, from })
}

/// Short, non-reversible label for a recipient — same reasoning as
/// `xindeler-auth`'s `audit_label`: logs should never carry raw PII.
fn redact(email: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(email.trim().to_lowercase().as_bytes());
    hex::encode(&digest.finalize()[..4])
}

fn plain_text_fallback(html: &str) -> String {
    // Cheap tag-stripping is enough for the alternative part — real mail
    // clients render the HTML part; this only exists so clients that can't
    // (or won't) show HTML have something readable, which the Python
    // templates never provided at all.
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The actual build-and-send, shared by both callers below. Kept separate
/// from `send_html_email` so the digest CLI subcommand (a short-lived
/// process — see `digest.rs`) can wait for delivery instead of racing a
/// background thread against process exit.
fn send_blocking(to: &str, subject: &str, html: &str) -> Result<(), String> {
    let Some(mailer) = MAILER.get().and_then(|slot| slot.as_ref()) else {
        return Err("mail not configured".to_owned());
    };

    let plain = plain_text_fallback(html);
    let message = Message::builder()
        .from(mailer.from.clone())
        .to(to
            .parse()
            .map_err(|_| "invalid recipient address".to_owned())?)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(plain),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html.to_owned()),
                ),
        )
        .map_err(|err| format!("failed to build email: {err}"))?;

    mailer
        .transport
        .send(&message)
        .map(|_| ())
        .map_err(|err| format!("delivery failed: {err}"))
}

/// Fire-and-forget, matching the Python service's `BackgroundTasks`
/// semantics: the caller (already running inside `spawn_blocking`) doesn't
/// wait for SMTP. Fase 1's volume is a handful of emails/hour at most
/// (capped by the endpoint's own rate limit) — a full retry/idempotency
/// queue (`xindeler-auth`'s `mail_queue.rs`) is infrastructure this doesn't
/// need yet; a failed send is logged and dropped, same as today.
pub fn send_html_email(to: String, subject: String, html: String) {
    std::thread::spawn(move || {
        if let Err(err) = send_blocking(&to, &subject, &html) {
            log::warn!("mail to {}: {err}", redact(&to));
        }
    });
}

/// Blocks until the send attempt completes (or fails) — used by the digest
/// CLI subcommand, which has no server loop to keep the process alive while
/// a background thread finishes.
pub fn send_html_email_blocking(to: &str, subject: &str, html: &str) -> Result<(), String> {
    send_blocking(to, subject, html)
}

/// Escapes user-controlled text before it's interpolated into an HTML email
/// template. The Python service this replaces interpolated `name`/`skills`/
/// `portfolio` raw — this is the fix.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Renders `portfolio` as a link only if it has a safe scheme — otherwise as
/// plain escaped text. The Python service put this field straight into an
/// `href` with no validation at all (a `javascript:`/`data:` injection
/// surface in the owner's mail client); this is the fix.
fn portfolio_html(portfolio: &str) -> String {
    let trimmed = portfolio.trim();
    if trimmed.is_empty() {
        return "<em>(sin portfolio)</em>".to_owned();
    }
    let escaped = escape_html(trimmed);
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        format!(r#"<a href="{escaped}" style="color:#c9a84c;">{escaped}</a>"#)
    } else {
        escaped
    }
}

pub(crate) fn wrap_email(content: &str, footer: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<body style="margin:0;padding:0;background:#060e1a;font-family:sans-serif;color:#e6e6e6;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#060e1a;padding:32px 0;">
<tr><td align="center">
<table role="presentation" width="480" cellpadding="0" cellspacing="0" style="background:#0d1b2a;border-radius:8px;overflow:hidden;">
<tr><td style="padding:0;">
<img src="https://xindeler.com/og-image.png" alt="Xindeler" width="480" style="display:block;width:100%;height:auto;">
</td></tr>
<tr><td style="padding:32px;">
{content}
</td></tr>
<tr><td style="padding:0 32px 32px;border-top:1px solid #1a2c3f;">
<p style="color:#7a8ba3;font-size:12px;margin-top:16px;">{footer}</p>
<p style="color:#7a8ba3;font-size:12px;"><a href="https://xindeler.com" style="color:#c9a84c;">xindeler.com</a> · <a href="https://github.com/Matute289/xindeler-new-horizon" style="color:#c9a84c;">GitHub</a></p>
</td></tr>
</table>
</td></tr>
</table>
</body>
</html>"#
    )
}

pub fn waitlist_html(name: &str) -> String {
    let name = escape_html(name.trim());
    wrap_email(
        &format!(
            r#"<h1 style="color:#c9a84c;font-size:20px;">¡Ya estás en la lista de espera!</h1>
<p>Hola {name}, gracias por sumarte a la lista de espera de Xindeler. Te vamos a avisar apenas tengamos novedades.</p>"#
        ),
        "Recibiste este email porque te registraste en la lista de espera de Xindeler.",
    )
}

pub fn contribute_user_html(name: &str) -> String {
    let name = escape_html(name.trim());
    wrap_email(
        &format!(
            r#"<h1 style="color:#c9a84c;font-size:20px;">¡Gracias por querer sumarte a Xindeler!</h1>
<p>Hola {name}, recibimos tu propuesta para contribuir al proyecto. Te vamos a escribir apenas la revisemos.</p>"#
        ),
        "Recibiste este email porque te postulaste como contribuidor de Xindeler.",
    )
}

pub fn contribute_owner_html(
    name: &str,
    email: &str,
    skills: &str,
    portfolio: &str,
    timestamp: &str,
) -> String {
    let name_e = escape_html(name.trim());
    let email_e = escape_html(email.trim());
    let skills_e = escape_html(skills.trim());
    let portfolio_html = portfolio_html(portfolio);
    let timestamp_e = escape_html(timestamp);
    wrap_email(
        &format!(
            r#"<h1 style="color:#c9a84c;font-size:20px;">Nuevo colaborador — {name_e}</h1>
<table style="width:100%;font-size:14px;">
<tr><td style="color:#7a8ba3;padding:4px 0;">Email</td><td>{email_e}</td></tr>
<tr><td style="color:#7a8ba3;padding:4px 0;">Skills</td><td>{skills_e}</td></tr>
<tr><td style="color:#7a8ba3;padding:4px 0;">Portfolio</td><td>{portfolio_html}</td></tr>
<tr><td style="color:#7a8ba3;padding:4px 0;">Fecha</td><td>{timestamp_e}</td></tr>
</table>"#
        ),
        "Notificación automática de nuevo contribuidor.",
    )
}
