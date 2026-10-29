//! Credentials, `WWW-Authenticate: Bearer` parsing and the token cache.
use std::fmt;

use secrecy::SecretString;

/// Registry credentials. `Debug` never prints the password.
#[derive(Clone)]
pub enum Credentials {
    Anonymous,
    Basic {
        username: String,
        password: SecretString,
    },
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!()
    }
}
