//! External inference engines for Local Embedded providers.
//!
//! LocalRouter does not ship engines. Users install them with their package
//! manager ([`recipes`]), or, for stable-diffusion.cpp, which no package
//! manager carries, LocalRouter downloads the latest release on the user's
//! click into its managed folder ([`download`], [`managed`]). We find the
//! engines ([`detect`]: chosen file, managed install, PATH), optionally run
//! the install for them ([`install`]), and launch, supervise and stop the
//! engine processes ([`supervisor`]).

pub mod detect;
pub mod download;
pub mod install;
pub mod managed;
pub mod platform;
pub mod process;
pub mod recipes;
pub mod supervisor;

pub use detect::{detect, resolve, EngineCommand, EngineSource, EngineStatus, LlamaCaps};
pub use install::{InstallError, InstallRunner, InstallSink, OutputStream};
pub use managed::ManagedInstall;
pub use platform::{Os, Platform};
pub use recipes::{InstallKind, RecipeId, KEV_GIT_REV};
pub use supervisor::{
    EngineError, EngineHandle, EngineProcessInfo, EngineState, LaunchSpec, Lease, PortArg,
    Supervisor,
};
