//! What this build is to the release service: the channel and version the
//! Release workflow published it as, stamped at build time
//! (`SPACEMAN_RELEASE_CHANNEL`, `SPACEMAN_RELEASE_VERSION`, passed through
//! `scripts/build-app.sh`). The stamp is compiled in rather than written to
//! `Info.plist`, because the bundle's `Info.plist` must stay byte-identical to
//! `app/Info.plist` and carry Cargo's version (`scripts/validate-release.sh`,
//! `.github/release/sign-macos.sh`). Builds from before the stamp fall back to
//! this updater's receipt, then Cargo's version.

use super::protocol::is_version_name;

const CHANNEL: Option<&str> = option_env!("SPACEMAN_RELEASE_CHANNEL");
const VERSION: Option<&str> = option_env!("SPACEMAN_RELEASE_VERSION");

/// The version this build was released as (the Release workflow's build-time
/// stamp), or Cargo's package version for local source builds.
pub fn build_version() -> &'static str {
    VERSION
        .filter(|version| is_version_name(version))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Identity {
    /// The channel this build was published on, when known. A channel setting
    /// that differs makes the update a switch the user confirms.
    pub(super) channel: Option<String>,
    pub(super) version: String,
}

/// This build's identity, from its stamp or, for builds from before it, the
/// receipt's version (a dev version means the dev channel).
pub(super) fn running(receipt_version: Option<&str>) -> Identity {
    identity(CHANNEL, VERSION, receipt_version, env!("CARGO_PKG_VERSION"))
}

pub(super) fn identity(
    channel: Option<&str>,
    version: Option<&str>,
    receipt_version: Option<&str>,
    cargo: &str,
) -> Identity {
    let version = version
        .filter(|version| is_version_name(version))
        .or(receipt_version)
        .unwrap_or(cargo);
    let channel = channel
        .filter(|channel| ["stable", "dev"].contains(channel))
        .map(str::to_owned)
        .or_else(|| version.contains("-dev").then(|| "dev".to_owned()));
    Identity {
        channel,
        version: version.to_owned(),
    }
}
