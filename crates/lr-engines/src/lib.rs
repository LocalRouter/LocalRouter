//! External inference engines for Direct providers.
//!
//! LocalRouter does not ship or download engines. Users install them with
//! their package manager ([`recipes`]); we find them on PATH ([`detect`]),
//! optionally run the install command for them ([`install`]), and launch,
//! supervise and stop the engine processes ([`supervisor`]).

pub mod detect;
pub mod install;
pub mod platform;
pub mod process;
pub mod recipes;
pub mod supervisor;

pub use detect::{detect, resolve, EngineCommand, EngineStatus, LlamaCaps};
pub use install::{InstallError, InstallRunner, InstallSink, OutputStream};
pub use platform::{Os, Platform};
pub use recipes::{RecipeId, KEV_GIT_REV};
pub use supervisor::{
    EngineError, EngineHandle, EngineProcessInfo, EngineState, LaunchSpec, Lease, PortArg,
    Supervisor,
};
