//! The volume explorer's core (spec 053): what a mount is, how a path inside one is validated,
//! the frames the server and the volume helper exchange, and the text and history rules. Nothing
//! here touches a filesystem or a container; the helper and the server are its adapters.

pub mod frame;
pub mod history;
pub mod lang;
pub mod mount;
pub mod op;
pub mod path;
pub mod text;
