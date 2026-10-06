# Scan response throughput regression (PF-06, RE-04)

The paired Linux release measurement at run 37481555261/source 62139404590d952119eacc275ab997ce9fa3297c completed successfully but showed approximately 7.7–8.1x scan time regression for 20k/200k wide fixtures. Successful measurement is not performance acceptance. Original receipt hashes were checked and raw metrics are retained in docs/benchmarks/linux_paired_release_37481555261.

The parent driver reads one fixed stdout and stderr block per poll. The runtime previously slept 20ms after every pending poll, including actual data consumption. Preserve bounded blocks, fair alternating streams, original response limits, per-block deadline/authorization/cancellation checks and original Job/leader/I/O ownership. Immediately schedule the next poll after actual nonempty output consumption; retain 20ms backoff when no output was consumed. No strict wall-clock or RSS bound is claimed.

## Observable acceptance

- A real child writing 256KiB stderr while accepting Request and producing valid stdout must complete under one second using the same next-poll delay as Runtime. Natural exit and original owner reaping remain required before the timing assertion. The original scheduling behavior failed at 1.691 seconds; this is behavioral RED.
- A real child holding End and both EOFs without leader exit must retain idle backoff and must not deliver a tree. Only natural exit permits delivery and original owner reaping.
- Existing driver cancellation, authorization denial, panic, malformed-frame and shared output-budget tests remain mandatory.
- Repeat the complete native paired release 20k/200k/deep/concurrent measurement for the actual new source before reporting performance recovery. Local fixture success is not full scan performance acceptance.
