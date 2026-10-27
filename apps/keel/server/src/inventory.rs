//! Inventories (spec 055 US4, data-model.md "Inventory").
//!
//! [`pin_latest`] is the seam runs use (US2, T069): it picks the revision a new run is pinned to
//! and the hosts that revision lists. US4's commands (T058) live alongside it.

use sqlx::PgConnection;
use uuid::Uuid;

/// A host as one inventory revision lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryHost {
    pub host: String,
    pub groups: Vec<String>,
}

/// The revision a run is pinned to, and its hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub revision: Uuid,
    pub hosts: Vec<InventoryHost>,
}

/// Why no revision could be pinned. Each is a run `state_reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinRefusal {
    /// No inventory by that name in the workspace, or it has no revisions yet.
    NotFound,
    /// The inventory is deprecated and accepts no new runs.
    Deprecated,
    /// The newest revision did not validate (`inventory_invalid`).
    Invalid,
}

impl PinRefusal {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotFound => "inventory_not_found",
            Self::Deprecated => "inventory_deprecated",
            Self::Invalid => "inventory_invalid",
        }
    }
}

/// Pins the newest revision of `name` in `workspace`. An invalid newest revision is refused, not
/// skipped: running against an older one would hide the error.
///
/// # Errors
///
/// The database failed; a refusal is `Ok(Err(_))`.
pub async fn pin_latest(
    conn: &mut PgConnection,
    workspace: Uuid,
    name: &str,
) -> Result<Result<Pinned, PinRefusal>, sqlx::Error> {
    let row: Option<(String, Option<Uuid>, Option<String>)> = sqlx::query_as(
        "select i.status, r.id, r.status from inventories i \
         left join lateral (select id, status from inventory_revisions \
           where inventory_id = i.id order by seq desc limit 1) r on true \
         where i.workspace_id = $1 and i.name = $2",
    )
    .bind(workspace)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?;
    let revision = match row {
        None | Some((_, None, _)) => return Ok(Err(PinRefusal::NotFound)),
        Some((status, _, _)) if status == "deprecated" => return Ok(Err(PinRefusal::Deprecated)),
        Some((_, Some(_), Some(rev_status))) if rev_status != "valid" => {
            return Ok(Err(PinRefusal::Invalid));
        }
        Some((_, Some(id), _)) => id,
    };
    let hosts: Vec<(String, Vec<String>)> = sqlx::query_as(
        "select host, groups from inventory_hosts where revision_id = $1 order by host",
    )
    .bind(revision)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Ok(Pinned {
        revision,
        hosts: hosts
            .into_iter()
            .map(|(host, groups)| InventoryHost { host, groups })
            .collect(),
    }))
}
