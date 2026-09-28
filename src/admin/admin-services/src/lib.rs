//! aka's read model: one use case that snapshots what `aka status` prints.
//!
//! There is no repository or database layer in this stack. aka is the
//! datastore — the docker engine plus `~/.aka/state.json` — so the use case
//! reads those two directly, through aka's own crates, exactly as the CLI does.

pub mod status;

pub use status::{GetStatusService, GetStatusTrait, GetStatusUseCase, PROXYD_FRESH_SECS};
