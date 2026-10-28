//! Instance settings: the enrolment snippet template and instructions (spec 060 US2, T029;
//! research R1).
//!
//! - One row in `instance_settings`, or none: with no row the default template and empty
//!   instructions apply, at version 0.
//! - `GetSettings`: instance owners and admins, from a session.
//! - `PreviewSettingsUpdate`: the same, with step-up. The template is validated
//!   ([`keel_core::enroll_template::validate`], its error code is the `OpError` code) and the
//!   instructions are capped at 20 KiB (`instructions_too_long`). Confirming re-checks admin,
//!   inserts the first row or updates `where version = $loaded`; a stale version is
//!   `settings_conflict`. The change is audited as `settings_update` with the text before and
//!   after.

use keel_core::enroll_template::{self, DEFAULT_TEMPLATE};
use keel_protocol::{Event, RoleMappingInfo};
use serde_json::json;
use sqlx::{PgConnection, PgPool};

use crate::config::Config;
use crate::dispatch::Request;
use crate::governance::{self, Fail, Step, check_admin, deny, origin};
use crate::identity::Caller;
use crate::preview::Draft;
use crate::serve::AppState;

/// The largest instructions accepted, in bytes (the column's check agrees).
pub const MAX_INSTRUCTIONS_BYTES: usize = 20 * 1024;

/// The settings as stored. `template` is `None` while none has been saved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stored {
    pub template: Option<String>,
    pub instructions: String,
    /// 0 while there is no row.
    pub version: i64,
}

impl Stored {
    /// The template in effect: the saved one, or the default.
    #[must_use]
    pub fn template(&self) -> &str {
        self.template.as_deref().unwrap_or(DEFAULT_TEMPLATE)
    }
}

type Row = (Option<String>, String, i64);

/// The stored settings, or the defaults when there is no row.
///
/// # Errors
///
/// The database read failed.
pub async fn load(db: &PgPool) -> Result<Stored, sqlx::Error> {
    let row: Option<Row> = sqlx::query_as(
        "select enroll_template, enroll_instructions, version from instance_settings",
    )
    .fetch_optional(db)
    .await?;
    Ok(stored(row))
}

fn stored(row: Option<Row>) -> Stored {
    row.map(|(template, instructions, version)| Stored {
        template,
        instructions,
        version,
    })
    .unwrap_or_default()
}

/// What `%s` renders to: the first runner host name and the runner listener's port.
#[must_use]
pub fn runner_address(cfg: &Config) -> Option<String> {
    let name = cfg.runner_host_names.first()?;
    let port = cfg.runner_listen.port();
    Some(if name.contains(':') {
        format!("[{name}]:{port}")
    } else {
        format!("{name}:{port}")
    })
}

/// What `%u` renders to: the public origin, without a trailing slash.
#[must_use]
pub fn public_url(cfg: &Config) -> String {
    cfg.origin()
}

/// The OIDC role mapping as shown to admins. None until the mapping is configurable
/// (spec 060 US5 fills this in).
#[must_use]
pub fn role_mapping_info(_app: &AppState) -> Option<RoleMappingInfo> {
    None
}

/// Warning codes for the settings page, such as `idp_names_no_owner`. Empty while no role
/// mapping is configured (spec 060 US5 adds that check).
///
/// # Errors
///
/// The database read failed.
pub async fn warnings(app: &AppState) -> Result<Vec<String>, sqlx::Error> {
    let _ = role_mapping_info(app);
    Ok(Vec::new())
}

async fn settings_event(app: &AppState) -> Vec<Event> {
    let result = async {
        let s = load(&app.db).await?;
        let warnings = warnings(app).await?;
        Ok::<_, sqlx::Error>((s, warnings))
    }
    .await;
    match result {
        Ok((s, warnings)) => vec![Event::Settings {
            template: s.template().to_owned(),
            is_default: s.template.is_none(),
            instructions: s.instructions,
            version: s.version,
            runner_address: runner_address(&app.cfg),
            public_url: public_url(&app.cfg),
            role_mapping: role_mapping_info(app),
            warnings,
        }],
        Err(e) => {
            tracing::error!(error = %e, "settings read failed");
            vec![governance::internal()]
        }
    }
}

/// `GetSettings`: instance admins.
pub async fn get(app: &AppState, req: &Request<'_>) -> Vec<Event> {
    const CMD: &str = "get_settings";
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, origin(req).context).await;
    }
    settings_event(app).await
}

/// Why a template or instructions can't be saved, as an `OpError`.
fn invalid(template: &str, instructions: &str) -> Option<Event> {
    if let Err(e) = enroll_template::validate(template) {
        let message = match e {
            enroll_template::TemplateError::MissingToken => {
                "The template must contain %t, the token.".to_owned()
            }
            enroll_template::TemplateError::UnknownPlaceholder => {
                "The template has an unknown placeholder. Use %t, %n, %p, %w, %s, %u, %e or %%."
                    .to_owned()
            }
            enroll_template::TemplateError::TooLong => format!(
                "The template is longer than {} bytes.",
                enroll_template::MAX_BYTES
            ),
        };
        return Some(Event::op_error(e.code(), message));
    }
    if instructions.len() > MAX_INSTRUCTIONS_BYTES {
        return Some(Event::op_error(
            "instructions_too_long",
            format!("The instructions are longer than {MAX_INSTRUCTIONS_BYTES} bytes."),
        ));
    }
    None
}

const CONFLICT: &str = "The settings changed since you loaded them. Reload and try again.";

/// `PreviewSettingsUpdate` (step-up). `Confirm` replies `Settings`, or `settings_conflict`.
pub async fn preview_update(
    app: &AppState,
    req: &Request<'_>,
    template: String,
    instructions: String,
    version: i64,
) -> Vec<Event> {
    const CMD: &str = "preview_settings_update";
    const ACTION: &str = "settings_update";
    let o = origin(req);
    if let Err(code) = check_admin(req.caller) {
        return deny(app, req.caller, CMD, None, code, o.context).await;
    }
    if let Some(e) = invalid(&template, &instructions) {
        return vec![e];
    }
    let summary = "Save the enrolment snippet template and instructions. Tokens minted from now \
                   on use them."
        .to_owned();
    let op = governance::pending(move |app, caller| async move {
        let step: Step = save(&app, &caller, &o, &template, &instructions, version).await;
        if let Err(events) = governance::settle(&app, &caller, ACTION, None, &o, step).await {
            return events;
        }
        settings_event(&app).await
    });
    let draft = Draft {
        action: ACTION.to_owned(),
        summary,
        affected: vec![],
        step_up: true,
    };
    governance::preview(app, req, draft, op)
}

async fn save(
    app: &AppState,
    caller: &Caller,
    o: &governance::Origin,
    template: &str,
    instructions: &str,
    version: i64,
) -> Step {
    const ACTION: &str = "settings_update";
    if let Err(code) = check_admin(caller) {
        return Ok(Err(Fail::Denied(code)));
    }
    let mut tx = app.db.begin().await?;
    let before = stored(
        sqlx::query_as(
            "select enroll_template, enroll_instructions, version from instance_settings \
             for update",
        )
        .fetch_optional(&mut *tx)
        .await?,
    );
    if before.version != version {
        return Ok(Err(Fail::Op("settings_conflict", CONFLICT.into())));
    }
    if !write(&mut tx, caller, template, instructions, version).await? {
        return Ok(Err(Fail::Op("settings_conflict", CONFLICT.into())));
    }
    let context = json!({
        "before": { "template": before.template(), "instructions": before.instructions },
        "after": { "template": template, "instructions": instructions },
        "version": version + 1,
        "channel": o.context["channel"],
    });
    governance::record(&mut tx, caller, ACTION, None, json!({}), &context).await?;
    tx.commit().await?;
    Ok(Ok(vec![]))
}

/// Inserts the first row (version 1) or updates the row at `version`. False: someone else
/// saved first.
async fn write(
    tx: &mut PgConnection,
    caller: &Caller,
    template: &str,
    instructions: &str,
    version: i64,
) -> Result<bool, sqlx::Error> {
    // `$4` is the version both ways, so the two statements bind alike.
    let sql = if version == 0 {
        "insert into instance_settings (enroll_template, enroll_instructions, updated_by) \
         select $1, $2, $3 where $4::bigint = 0 on conflict (id) do nothing"
    } else {
        "update instance_settings set enroll_template = $1, enroll_instructions = $2, \
         updated_by = $3, updated_at = now(), version = version + 1 where version = $4"
    };
    let done = sqlx::query(sql)
        .bind(template)
        .bind(instructions)
        .bind(caller.user.id)
        .bind(version)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    Ok(done == 1)
}
