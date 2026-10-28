//! Opt-in developer measurements. Never print paths, URLs, accounts or errors.
use std::time::Instant;

#[derive(Clone, Copy)]
pub(super) enum Stage {
    Total,
    Catalog,
    Metadata,
    RuntimeReservation,
    Capacity,
    Cache,
    Dependencies,
    Java,
    Loader,
    Natives,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Total => "total",
            Self::Catalog => "catalog",
            Self::Metadata => "metadata",
            Self::RuntimeReservation => "runtime_reservation",
            Self::Capacity => "capacity",
            Self::Cache => "cache",
            Self::Dependencies => "dependencies",
            Self::Java => "java",
            Self::Loader => "loader",
            Self::Natives => "natives",
        }
    }
}

pub(super) struct Timer {
    stage: Stage,
    started: Option<Instant>,
}

impl Timer {
    pub(super) fn new(stage: Stage) -> Self {
        Self {
            stage,
            started: (std::env::var_os("PANITA_INSTALL_TIMINGS").as_deref()
                == Some(std::ffi::OsStr::new("1")))
            .then(Instant::now),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            // A duration describes time spent, including failed/cancelled work;
            // it does not imply that the stage or installation succeeded.
            eprintln!(
                "Panita installation timing: stage={} elapsed_ms={}",
                self.stage.name(),
                started.elapsed().as_millis()
            );
        }
    }
}
