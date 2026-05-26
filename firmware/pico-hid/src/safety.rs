use crate::dispatch::DispatchState;

pub const DEFAULT_WATCHDOG_TIMEOUT_MS: u32 = 1000;
pub const WATCHDOG_DISABLED_TIMEOUT_MS: u32 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchdogPoll {
    Noop,
    Fired,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Watchdog {
    timeout_ms: u32,
    last_valid_command_ms: u32,
    fired: bool,
}

impl Watchdog {
    pub const fn new() -> Self {
        Self {
            timeout_ms: DEFAULT_WATCHDOG_TIMEOUT_MS,
            last_valid_command_ms: 0,
            fired: false,
        }
    }

    pub fn record_valid_command(&mut self, now_ms: u32, timeout_ms: u32) {
        self.timeout_ms = timeout_ms;
        self.last_valid_command_ms = now_ms;
        self.fired = false;
    }

    pub fn poll(&mut self, now_ms: u32, state: &mut DispatchState) -> WatchdogPoll {
        if self.timeout_ms == WATCHDOG_DISABLED_TIMEOUT_MS {
            return WatchdogPoll::Disabled;
        }

        if self.fired {
            return WatchdogPoll::Noop;
        }

        if now_ms.wrapping_sub(self.last_valid_command_ms) < self.timeout_ms {
            return WatchdogPoll::Noop;
        }

        state.release_all();
        state.telemetry.watchdog_fires = state.telemetry.watchdog_fires.wrapping_add(1);
        self.fired = true;
        WatchdogPoll::Fired
    }

    pub const fn timeout_ms(self) -> u32 {
        self.timeout_ms
    }

    pub const fn fired(self) -> bool {
        self.fired
    }
}

impl Default for Watchdog {
    fn default() -> Self {
        Self::new()
    }
}
