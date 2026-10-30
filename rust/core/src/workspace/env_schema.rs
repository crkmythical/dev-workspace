//! The user-facing environment as DATA — a partial port of
//! packages/core/src/workspace/env-schema.ts.
//!
//! Only the *types* are ported here. The big `CORE_ENV_GROUPS` data table is consumed solely by
//! the `.env.example` generator and the drift test, both of which stay in TypeScript until stage
//! 3; the plugin contract (crate::plugins) carries env groups, so it needs these shapes now.

/// One environment variable: its name, the comment written above it in the example file, and the
/// placeholder shown for it. Secrets are left empty; a test rejects anything that looks like a
/// real token. The three optional TS fields collapse to a `bool`/`bool`/`Option` with the same
/// "absent means off" meaning.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvVarSpec {
    pub name: String,
    /// Comment lines written above the variable.
    pub doc: Vec<String>,
    /// Value shown in the example file. A placeholder, never a credential.
    pub example: String,
    /// Written as `# NAME=value`: the feature stays off until the user opts in.
    pub opt_in: bool,
    /// The container refuses to start without it.
    pub required: bool,
    /// Read by a third-party tool rather than by this repo (name it); exempt from the
    /// dead-variable check.
    pub external: Option<String>,
}

/// A titled group of variables, written as one section of `.env.example`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvGroup {
    pub title: String,
    pub vars: Vec<EnvVarSpec>,
}
