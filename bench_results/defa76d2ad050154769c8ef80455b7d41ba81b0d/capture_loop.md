# capture_loop bench result

Commit: `defa76d2ad050154769c8ef80455b7d41ba81b0d`
Date: 2026-05-23

## Windows GetProcessTimes run

Command:

```text
SYNAPSE_CAPTURE_BENCH_SECONDS=30 C:\Temp\synapse-bench\capture_loop.exe --quiet --noplot
```

Result:

```text
capture_loop_steady_state source=GetProcessTimes duration_secs=30.000 cpu_percent=0.0423 frames_captured=1431 frames_dropped=0 frames_consumed=1431 channel_len=0
```

Budget: capture with consumer attached at 60 fps must be `<= 2%` normalized CPU.
Observed normalized CPU: `0.0423%`.

## WSL synthetic readback run

Command:

```text
SYNAPSE_CAPTURE_BENCH_SECONDS=30 cargo bench -p synapse-capture --bench capture_loop -- --quiet
```

Result:

```text
capture_loop_60fps_start_stop time: [66.151 ms 66.486 ms 66.777 ms]
capture_loop_steady_state source=/proc/self/stat duration_secs=30.002 cpu_percent=0.0111 frames_captured=1796 frames_dropped=0 frames_consumed=1796 channel_len=0
```
