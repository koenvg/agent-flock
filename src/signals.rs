#[cfg(unix)]
use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
#[cfg(unix)]
use signal_hook::{SigId, low_level};
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

pub struct SignalMonitor {
    pending: Arc<AtomicI32>,
    #[cfg(unix)]
    registrations: Vec<SigId>,
}

impl SignalMonitor {
    pub fn install() -> io::Result<Self> {
        let pending = Arc::new(AtomicI32::new(0));
        let mut registrations = Vec::new();

        for signal in [SIGHUP, SIGINT, SIGQUIT, SIGTERM] {
            let pending_for_handler = Arc::clone(&pending);
            let registration = unsafe {
                low_level::register(signal, move || {
                    pending_for_handler.store(signal, Ordering::SeqCst);
                })
            };
            match registration {
                Ok(registration) => registrations.push(registration),
                Err(error) => {
                    for registration in registrations {
                        low_level::unregister(registration);
                    }
                    return Err(error);
                }
            }
        }

        Ok(Self {
            pending,
            registrations,
        })
    }

    pub fn take(&self) -> Option<i32> {
        match self.pending.swap(0, Ordering::SeqCst) {
            0 => None,
            signal => Some(signal),
        }
    }

    pub fn terminate(self, signal: i32) -> ! {
        drop(self);
        let _ = low_level::emulate_default_handler(signal);
        std::process::exit(128 + signal);
    }
}

impl Drop for SignalMonitor {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            low_level::unregister(registration);
        }
    }
}
