//! Samples machine load while a local model runs.
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use sysinfo::System;

#[derive(Clone, Copy, Debug, Default)]
pub struct HwPeak {
    pub cpu_percent: f32,
    pub ram_used_mb: f64,
    /// NVIDIA GPU utilisation, when `nvidia-smi` is available.
    pub gpu_percent: Option<f32>,
}

pub struct Sampler {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<HwPeak>>,
}

impl Sampler {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let mut sys = System::new();
            let mut peak = HwPeak::default();
            let mut gpu_available = true;
            sys.refresh_cpu_usage(); // first reading is a baseline
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(500));
                sys.refresh_cpu_usage();
                sys.refresh_memory();
                peak.cpu_percent = peak.cpu_percent.max(sys.global_cpu_usage());
                peak.ram_used_mb = peak.ram_used_mb.max(sys.used_memory() as f64 / 1_048_576.0);
                if gpu_available {
                    match nvidia_gpu_percent() {
                        Some(g) => peak.gpu_percent = Some(peak.gpu_percent.unwrap_or(0.0).max(g)),
                        None => gpu_available = false,
                    }
                }
            }
            peak
        });
        Self { stop, handle: Some(handle) }
    }

    pub fn finish(mut self) -> HwPeak {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().and_then(|h| h.join().ok()).unwrap_or_default()
    }
}

/// A run that is cancelled drops its sampler without calling `finish`; the
/// thread must still stop, or it would poll the machine until the app exits.
impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn nvidia_gpu_percent() -> Option<f32> {
    let mut cmd = Command::new("nvidia-smi");
    cmd.args(["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse::<f32>().ok()).reduce(f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_a_sampler_stops_its_thread() {
        let sampler = Sampler::start();
        let stop = Arc::clone(&sampler.stop);
        drop(sampler);
        assert!(stop.load(Ordering::Relaxed));
    }

    #[test]
    fn finish_returns_a_reading() {
        let peak = Sampler::start().finish();
        assert!(peak.cpu_percent >= 0.0);
    }
}
