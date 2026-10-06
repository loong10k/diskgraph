# Windows internal birth API retirement

Native release packaging at source e7f16ff/job 112345452135 failed strict production compilation because three crate-private WindowsChild compatibility birth functions and ChildSpawnError::with_cleanup had no production callers. Current product probes already use spawn_into_with_admission and retain the original prepared Job/process/I/O in the caller recovery boundary. The obsolete functions cannot transfer failed ownership through their returned signature.

Retire the obsolete production functions rather than hiding them behind a test flag or suppressing warnings. Keep the single Windows product birth API with explicit caller-owned slot, admission and lifecycle checks. No public Rust/CLI/MCP/FFI signature changes. Cleanup composition used by actual Unix production launchers remains Unix-specific.

Migrate native fixture call sites to WindowsTestBirth, a finite test harness using the actual product birth API. Its rescue owner lives outside catch; original typed checkpoint error and panic payload remain unchanged. Tests needing retained ownership continue to own their external slot. The twenty-second fixture rescue bound does not replace product request limits or the existing fixture checkpoints. Existing assertion macro counts were checked without weakening their assertions.

Require actual Windows stable and MSRV production compilation, original native birth/control/policy/cleanup cases, driver and host regressions and package validation. Local macOS compilation and driver/source-layout checks do not prove Windows support. Platform parents remain open.

## Native regression found at 0712cdd

Windows stable and MSRV unit compilation exposed three missed test references: the platform alias still used the retired birth method, and two ProbeFailure projections still called the Unix-only cleanup composer. Route the Windows checkpoint fixture through WindowsTestBirth and explicitly compose the Windows caller-owned checkpoint/cleanup error. Preserve the original typed error, original OS error, cancellation classification and exact diagnostic assertions. Unix still exercises its real production cleanup method. Native package release compilation succeeded at this source, but packaged execution still returned Unsupported; this is not Windows product acceptance.

## Resumed loader qualification

Build a fresh valid unsigned DLL using the installed MSVC toolchain. After actual child resume, WorkerControl must reject it with ERROR_INVALID_IMAGE_HASH while loading the actual System32 version.dll. Null mode must load the valid unsigned DLL and resolve its exported marker. Require natural leader exit, drained output and zero active processes in the original Job, preserve the DLL digest, and archive both original native test markers. This verifies loader policy behavior only; it does not close dependency trust, bound scan image execution or Windows Runtime integration.
