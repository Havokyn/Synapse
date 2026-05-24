use std::time::{Duration, Instant};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        System::Threading::{
            AVRT_PRIORITY_CRITICAL, AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
            AvSetMmThreadPriority, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW,
            GetCurrentThread, INFINITE, SetThreadPriority, SetWaitableTimerEx,
            THREAD_PRIORITY_TIME_CRITICAL, TIMER_ALL_ACCESS, WaitForSingleObject,
        },
    },
    core::{PCWSTR, w},
};

pub struct WindowsHighResolutionTimer {
    timer: HANDLE,
    mmcss: HANDLE,
}

// The configured Win11 host shows >200us p99 wake jitter if the final
// millisecond is left entirely to the kernel timer. Longer intervals still
// park on the high-resolution timer until this precision window.
const SPIN_WINDOW: Duration = Duration::from_millis(1);

impl WindowsHighResolutionTimer {
    pub fn start(target_interval: Duration) -> Result<Self, String> {
        // SAFETY: GetCurrentThread returns a pseudo-handle for the current thread,
        // and SetThreadPriority only mutates that thread's scheduler priority.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) }
            .map_err(|error| format!("SetThreadPriority TIME_CRITICAL failed: {error}"))?;

        let mut task_index = 0_u32;
        // SAFETY: The task name is a static null-terminated UTF-16 literal and
        // task_index is initialized to 0 as required for the first MMCSS call.
        let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &raw mut task_index) }
            .map_err(|error| format!("MMCSS Pro Audio registration failed: {error}"))?;
        // SAFETY: mmcss is the task handle returned by AvSetMmThreadCharacteristicsW.
        if let Err(error) = unsafe { AvSetMmThreadPriority(mmcss, AVRT_PRIORITY_CRITICAL) } {
            // SAFETY: mmcss was returned by AvSetMmThreadCharacteristicsW above.
            let _ = unsafe { AvRevertMmThreadCharacteristics(mmcss) };
            return Err(format!("MMCSS critical priority failed: {error}"));
        }

        // SAFETY: Null security attributes/name create a private unnamed timer.
        // The returned handle is owned by this guard and closed in Drop.
        let timer = match unsafe {
            CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
        } {
            Ok(timer) => timer,
            Err(error) => {
                // SAFETY: mmcss was returned by AvSetMmThreadCharacteristicsW above.
                let _ = unsafe { AvRevertMmThreadCharacteristics(mmcss) };
                return Err(format!(
                    "CreateWaitableTimerExW high-resolution failed: {error}"
                ));
            }
        };

        if let Err(error) = arm_timer(timer, target_interval) {
            // SAFETY: handles are valid and owned by this function on this path.
            let _ = unsafe { CloseHandle(timer) };
            let _ = unsafe { AvRevertMmThreadCharacteristics(mmcss) };
            return Err(error);
        }

        Ok(Self { timer, mmcss })
    }

    pub fn wait_until(&self, deadline: Instant) -> Result<(), String> {
        let wait = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        let timer_wait = wait.saturating_sub(SPIN_WINDOW);
        if !timer_wait.is_zero() {
            arm_timer(self.timer, timer_wait)?;
            // SAFETY: self.timer is a live waitable timer handle owned by this guard.
            let result = unsafe { WaitForSingleObject(self.timer, INFINITE) };
            if result != WAIT_OBJECT_0 {
                return Err(format!(
                    "WaitForSingleObject on scheduler timer returned {result:?}"
                ));
            }
        }
        while Instant::now() < deadline {
            std::hint::spin_loop();
        }
        Ok(())
    }
}

impl Drop for WindowsHighResolutionTimer {
    fn drop(&mut self) {
        // SAFETY: both handles were acquired by this guard and are dropped once.
        let _ = unsafe { CloseHandle(self.timer) };
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.mmcss) };
    }
}

fn duration_100ns(duration: Duration) -> i64 {
    let ticks = duration.as_nanos() / 100;
    i64::try_from(ticks).unwrap_or(i64::MAX)
}

fn arm_timer(timer: HANDLE, duration: Duration) -> Result<(), String> {
    let due_time = -duration_100ns(duration);
    // SAFETY: timer is a valid waitable timer handle, due_time points to a
    // live i64 for the duration of the call, and no APC callback is used.
    unsafe { SetWaitableTimerEx(timer, &raw const due_time, 0, None, None, None, 0) }
        .map_err(|error| format!("SetWaitableTimerEx one-shot failed: {error}"))
}
