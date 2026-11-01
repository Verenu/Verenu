# Linux memory investigation

These measurements describe an isolated test workload, not a guaranteed saving
for every installation. They use release builds, actual main and pill windows,
WebKitGTK 2.54.2, an AMD render device, and a database with 2,000 generated
history entries. Each comparison alternated five fresh launches of the two
variants. PSS includes proportional shares of resident shared pages and excludes
swap. Neither a GPU device handle nor a successful window render alone proves
that hardware acceleration is being used.

## Implemented changes

- **Database cache:** On Linux with glibc, after two consecutive maintenance
  checks find the main window hidden and dictation idle, Verenu asks SQLite to
  discard unused cache pages and returns free allocator pages to the OS. The
  database lock is never waited for during this cleanup. SQLite reads pages
  again when needed; reopening the app can therefore incur extra disk reads.
  In five paired runs, the backend's PSS change from approximately 40 seconds
  to 75 seconds after hiding the window had a median of +41 KiB without this
  change (range -105 to +126 KiB) and -1,854 KiB with it (range -2,224 to
  -1,376 KiB): a difference of **1,895 KiB (1.85 MiB)**. Total process-tree
  PSS varied too much between launches to establish a separate total saving.
  Results depend on history activity, cache warmth, and memory pressure.
- **Offline indicator:** Linux uses a static offline dot instead of a
  continuous pulse. In five paired runs, idle app and WebKit CPU fell from a
  median 9.0% to 0.6% of one core, with no measurable memory reduction.
  Windows and macOS retain the pulse.

## Configurations investigated

Sharing the WebKit renderer reduced median process-tree PSS by about 57 MiB in
this workload, but killing that renderer blanked both windows and neither
recovered during a 15-second observation. It was not adopted because it couples
the windows' failure and security domains. DMA-BUF versus shared-memory and
forced versus automatic compositing comparisons did not establish safe,
repeatable savings across rendering and hardware configurations. Servo is not
an existing Verenu runtime and was not benchmarked as a replacement.

Disabling JavaScriptCore's JIT reduced median process-tree PSS from 347.03 MiB
to 325.39 MiB across five interleaved launches of the same synthetic workload.
That setting is not enabled by Verenu: short navigation checks do not establish
the impact on dictation, pill responsiveness, complex views, or other machines.
It would also rely on a WebKit runtime option whose behavior can change between
versions.

Disabling only JavaScriptCore's concurrent JIT compilation did not produce a
measurable improvement in this workload: five launches per setting had median
process-tree PSS of 338.21 MiB with the default (range 336.08–346.12 MiB) and
338.69 MiB with concurrent compilation disabled (range 337.34–340.32 MiB).
The JIT setting was left unchanged. These launch-to-launch results do not prove
the option's effect on interaction latency or other workloads.

An experimental WebKitGTK memory-pressure configuration used a 128 MiB limit,
the default cleanup thresholds and poll interval, and no custom kill threshold.
It was injected into the context constructor only for the experiment. Five
launches per setting gave median process-tree PSS of 341.35 MiB with default
settings (range 334.19–369.69 MiB) and 338.54 MiB with the lower limit (range
329.14–370.02 MiB). The median idle CPU over 35-second samples increased from
2.14% to 4.00% of one core. Shared file-backed PSS varied markedly between
launches; renderer anonymous PSS also varied. The experiment does not establish
a repeatable total saving or interaction safety in the idle workload.

After three Insights/Home navigation cycles, five interleaved pairs with the
WebKit inspector attached showed median process-tree PSS of 358.67 MiB with
default settings (range 353.54–361.16 MiB) and 327.32 MiB with the lower limit
(range 324.63–332.51 MiB). The median paired reduction was 29.59 MiB; median
idle CPU rose from 1.51% to 3.11% of one core. Both renderer processes held the
GPU render-device file descriptor, but this series did not directly measure GPU
execution. A preliminary run with the same workload produced an inconsistent
anonymous-PSS result, and the inspector itself may affect memory.

A separate five-pair run used screenshots and OCR to verify the same navigation
without the inspector. Median process-tree PSS was 301.08 MiB with default
settings (range 290.03–352.85 MiB) and 281.94 MiB with the lower limit (range
262.92–326.61 MiB). Paired reductions ranged from 14.80 to 33.86 MiB, with a
median of 26.23 MiB. Median idle CPU rose from 2.20% to 3.34% of one core.
Both renderers held the AMD render device in every sample; one observed control
renderer had accumulated graphics-engine time. These results do not establish
interaction latency, real dictation, or crash-recovery safety. With the policy
injected into a native debug build, the pill lifecycle test passed for sizing,
state transitions, input policy, and stale-state rejection in an isolated
desktop. A separate release-build probe killed one renderer: the surviving
main window opened Insights, but the killed renderer did not restart during
the observation. Five alternating pairs of native debug sessions also measured
20 synthetic pill state transitions each through WebDriver. The median of the
per-session medians for the 19 warmed transitions was 9.87 ms with default
settings and 9.62 ms with the lower limit; the largest warmed observations were
13.57 ms and 12.67 ms respectively. First transitions, which included setup,
had medians of 1,023 and 1,028 ms. This measures test-driver invocation through
DOM state detection, not actual paint, shortcuts, audio, or dictation latency;
the sessions did not repeat the navigation workload. Verenu does not enable the
experimental policy.

An exploratory JavaScript heap profile followed six Insights visits in one
release session. Full-collection snapshots estimated 2.86 MiB of live heap
objects before the first visit, 4.37 MiB after one visit, and 4.57 MiB after
six. The first visit loaded additional code; the later increase included code
blocks as well as strings and objects. Combined renderer anonymous PSS before
the snapshots grew by about 45 MiB from the initial sample to the last, so the
live objects represented by these snapshots alone do not account for that
resident increase. One idle control with the same snapshot schedule grew from
2.86 to 3.47 MiB of estimated live objects and about 1.9 MiB of renderer
anonymous PSS. The later snapshot-to-snapshot live-object growth was similar in
both sessions, so some growth may come from the measurement itself. These are
single instrumented sessions, not a leak test or an estimate of a saving:
snapshots do not include all native and GPU allocations. No frontend change was
made on this evidence.

The test desktop did not exercise a real microphone, global shortcut, live
provider, or the user's GPU configuration. These results do not establish their
behavior or a full cross-platform regression pass.
