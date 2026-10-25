//! A volume helper for tests (spec 053): the real helper's `serve` on a thread over pipes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use depths_core::ports::{HelperChannel, HelperProcess, HelperSpec, RuntimeError, VolumeHelper};
use depths_protocol::{MountId, MountKind};
use depths_runtime_cli::helper::{HELPER_IDLE, HelperPool, SystemClock};
use depths_volumes::mount::HelperSource;

use super::App;

/// Runs the real helper's `serve` on a thread over pipes, rooted at the folder its source maps
/// to, and records every spec it was started with.
#[derive(Default)]
pub struct FakeVolumeHelper {
    pub roots: HashMap<String, PathBuf>,
    pub specs: Mutex<Vec<HelperSpec>>,
}

struct Thread(Option<JoinHandle<i32>>);

impl HelperProcess for Thread {
    fn close(mut self: Box<Self>) {
        if let Some(t) = self.0.take() {
            let _ = t.join();
        }
    }
    fn kill(self: Box<Self>) {}
}

impl VolumeHelper for FakeVolumeHelper {
    fn start(&self, spec: &HelperSpec) -> Result<HelperChannel, RuntimeError> {
        self.specs.lock().unwrap().push(spec.clone());
        let dir = match &spec.source {
            HelperSource::Volume(name) => self.roots.get(name).cloned(),
            HelperSource::Bind(path) => Some(path.clone()),
        }
        .ok_or(RuntimeError::Unsupported)?;
        let root: std::os::fd::OwnedFd = std::fs::File::open(dir).unwrap().into();
        let (in_r, in_w) = std::io::pipe().unwrap();
        let (out_r, out_w) = std::io::pipe().unwrap();
        let t = std::thread::spawn(move || depths_volume_helper::serve(root, in_r, out_w));
        Ok(
            HelperChannel::open(Box::new(out_r), Box::new(in_w), Box::new(Thread(Some(t))))
                .unwrap(),
        )
    }

    fn volume_exists(&self, name: &str) -> Result<bool, RuntimeError> {
        Ok(self.roots.contains_key(name))
    }
}

impl FakeVolumeHelper {
    pub fn specs(&self) -> Vec<HelperSpec> {
        self.specs.lock().unwrap().clone()
    }
}

/// The volume the `valheim` stack mounts.
pub const VOLUME: &str = "valheim_saves";

/// Gives `a` a `valheim` stack whose running `valheim` service mounts volume `valheim_saves`
/// read-write, backed by `root`, and a helper pool over the fake. Returns the helper and the
/// mount's id.
pub fn install(a: &App, root: PathBuf) -> (Arc<FakeVolumeHelper>, MountId) {
    let helper = Arc::new(FakeVolumeHelper {
        roots: HashMap::from([(VOLUME.to_owned(), root)]),
        ..Default::default()
    });
    let pool = HelperPool::new(helper.clone(), Arc::new(SystemClock), 8, HELPER_IDLE);
    a.state.files.install(helper.clone(), pool);
    a.set_containers(vec![super::container(
        "valheim",
        "valheim",
        "running",
        "Up 1 hour",
    )]);
    a.set_inspect(
        "valheim-valheim-id",
        serde_json::json!({"Mounts": [{
            "Type": "volume", "Name": VOLUME, "Source": "/x",
            "Destination": "/config", "RW": true,
        }]}),
    );
    (helper, MountId::compute(MountKind::Volume, VOLUME))
}
