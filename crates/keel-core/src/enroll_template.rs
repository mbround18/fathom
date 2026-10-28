//! The enrolment snippet template (spec 060, FR-008 to FR-010). Pure: validate and render only.
//!
//! Placeholders are `%t` token, `%n` inventory name, `%p` profiles, `%w` workspace slug, `%s`
//! runner address, `%u` public URL and `%e` expiry; `%%` is a literal `%`. Rendering is a single
//! left-to-right pass, so a substituted value is never scanned again.

/// The template used while no one has saved one.
pub const DEFAULT_TEMPLATE: &str =
    "read -rs KEEL_TOKEN; export KEEL_TOKEN  # paste: %t\nmake keel-node";

/// The largest template accepted, in bytes.
pub const MAX_BYTES: usize = 4096;

/// Why a template was refused. The `code` is stable and goes into `OpError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("the template must contain %t")]
    MissingToken,
    #[error("the template has an unknown placeholder")]
    UnknownPlaceholder,
    #[error("the template is longer than 4096 bytes")]
    TooLong,
}

impl TemplateError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            TemplateError::MissingToken => "missing_token",
            TemplateError::UnknownPlaceholder => "unknown_placeholder",
            TemplateError::TooLong => "too_long",
        }
    }
}

/// What each placeholder renders to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Values {
    /// `%t`: the enrolment token.
    pub token: String,
    /// `%n`: the host's inventory name.
    pub name: String,
    /// `%p`: the profiles, comma-separated.
    pub profiles: String,
    /// `%w`: the workspace slug.
    pub workspace: String,
    /// `%s`: the runner address, `host:port`.
    pub runner: String,
    /// `%u`: the public URL.
    pub url: String,
    /// `%e`: the expiry, RFC 3339.
    pub expires: String,
}

impl Values {
    fn get(&self, c: char) -> Option<&str> {
        Some(match c {
            't' => &self.token,
            'n' => &self.name,
            'p' => &self.profiles,
            'w' => &self.workspace,
            's' => &self.runner,
            'u' => &self.url,
            'e' => &self.expires,
            _ => return None,
        })
    }
}

/// Checks a template: length first, then placeholders, then that `%t` is present.
pub fn validate(template: &str) -> Result<(), TemplateError> {
    if template.len() > MAX_BYTES {
        return Err(TemplateError::TooLong);
    }
    let mut token = false;
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            continue;
        }
        match chars.next() {
            Some('%') => {}
            Some('t') => token = true,
            Some('n' | 'p' | 'w' | 's' | 'u' | 'e') => {}
            _ => return Err(TemplateError::UnknownPlaceholder),
        }
    }
    if token {
        Ok(())
    } else {
        Err(TemplateError::MissingToken)
    }
}

/// Renders a template in one pass. An unknown placeholder or a trailing `%` is copied as-is;
/// `validate` refuses those before a template is saved.
#[must_use]
pub fn render(template: &str, values: &Values) -> String {
    let mut out = String::with_capacity(template.len() + values.token.len());
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(p) => match values.get(p) {
                Some(v) => out.push_str(v),
                None => {
                    out.push('%');
                    out.push(p);
                }
            },
            None => out.push('%'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../specs/060-keel-enroll-settings/contracts/template-vectors.json"
        );
        let text = std::fs::read_to_string(path).expect("template vectors");
        serde_json::from_str(&text).expect("vectors parse")
    }

    fn values(v: &Value) -> Values {
        let s = |k: &str| v[k].as_str().expect(k).to_owned();
        Values {
            token: s("t"),
            name: s("n"),
            profiles: s("p"),
            workspace: s("w"),
            runner: s("s"),
            url: s("u"),
            expires: s("e"),
        }
    }

    fn str_of(v: &Value) -> &str {
        v.as_str().expect("string")
    }

    #[test]
    fn render_vectors() {
        let v = vectors();
        let vals = values(&v["values"]);
        for case in v["render"].as_array().expect("render") {
            let template = str_of(&case["template"]);
            assert_eq!(validate(template), Ok(()), "{template:?}");
            assert_eq!(
                render(template, &vals),
                str_of(&case["out"]),
                "{template:?}"
            );
        }
    }

    #[test]
    fn single_pass_vector() {
        let v = &vectors()["single_pass"];
        let template = str_of(&v["template"]);
        assert_eq!(render(template, &values(&v["values"])), str_of(&v["out"]));
    }

    #[test]
    fn refuse_vectors() {
        let v = vectors();
        for case in v["refuse"].as_array().expect("refuse") {
            let template = str_of(&case["template"]);
            let err = validate(template).expect_err(template);
            assert_eq!(err.code(), str_of(&case["code"]), "{template:?}");
        }
    }

    #[test]
    fn too_long_vector() {
        let v = &vectors()["too_long"];
        let bytes = usize::try_from(v["bytes"].as_u64().expect("bytes")).expect("fits");
        let template = format!("%t{}", "x".repeat(bytes - 2));
        assert_eq!(template.len(), bytes);
        assert_eq!(
            validate(&template).map_err(|e| e.code()),
            Err(str_of(&v["code"]))
        );
        // Exactly at the cap is fine, and the length check comes before the others.
        assert_eq!(validate(&template[..MAX_BYTES]), Ok(()));
        assert_eq!(validate(&"%x".repeat(3000)), Err(TemplateError::TooLong));
    }

    #[test]
    fn default_template_is_valid() {
        assert_eq!(validate(DEFAULT_TEMPLATE), Ok(()));
    }

    #[test]
    fn unknown_placeholder_comes_before_missing_token() {
        assert_eq!(validate("%x"), Err(TemplateError::UnknownPlaceholder));
        assert_eq!(validate("make %"), Err(TemplateError::UnknownPlaceholder));
    }
}
