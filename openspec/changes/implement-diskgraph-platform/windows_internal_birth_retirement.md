# Windows internal birth API retirement

Native release packaging at source e7f16ff/job 112345452135 failed strict production compilation because three crate-private WindowsChild compatibility birth functions and ChildSpawnError::with_cleanup had no production callers. Current product probes already use spawn_into_with_admission and retain the original prepared Job/process/I/O in the caller recovery boundary. The obsolete functions cannot transfer failed ownership through their returned signature.

Retire the obsolete production functions rather than hiding them behind a test flag or suppressing warnings. Keep the single Windows product birth API with explicit caller-owned slot, admission and lifecycle checks. No public Rust/CLI/MCP/FFI signature changes. Cleanup composition used by actual Unix production launchers remains Unix-specific.

Migrate native fixture call sites to WindowsTestBirth, a finite test harness using the actual product birth API. Its rescue owner lives outside catch; original typed checkpoint error and panic payload remain unchanged. Tests needing retained ownership continue to own their external slot. The twenty-second fixture rescue bound does not replace product request limits or the existing fixture checkpoints. Existing assertion macro counts were checked without weakening their assertions.

Require actual Windows stable and MSRV production compilation, original native birth/control/policy/cleanup cases, driver and host regressions and package validation. Local macOS compilation and driver/source-layout checks do not prove Windows support. Platform parents remain open.

## Native regression found at 0712cdd

Windows stable and MSRV unit compilation exposed three missed test references: the platform alias still used the retired birth method, and two ProbeFailure projections still called the Unix-only cleanup composer. Route the Windows checkpoint fixture through WindowsTestBirth and explicitly compose the Windows caller-owned checkpoint/cleanup error. Preserve the original typed error, original OS error, cancellation classification and exact diagnostic assertions. Unix still exercises its real production cleanup method. Native package release compilation succeeded at this source, but packaged execution still returned Unsupported; this is not Windows product acceptance.

## Resumed loader qualification

Build a fresh valid unsigned DLL using the installed MSVC toolchain. After actual child resume, WorkerControl must reject it with ERROR_INVALID_IMAGE_HASH while loading the actual System32 version.dll. Null mode must load the valid unsigned DLL and resolve its exported marker. Require natural leader exit, drained output and zero active processes in the original Job, preserve the DLL digest, and archive both original native test markers. This verifies loader policy behavior only; it does not close dependency trust, bound scan image execution or Windows Runtime integration.

## Actual scan worker protocol gate

Before enabling Windows Runtime, execute the current Cargo-built diskgraph-scan-worker using the existing native WorkerControl birth and production ScanWorkerDriver in isolated directories. Bind the artifact to the current checkout and its actual SHA256/size from Cargo JSON; never discover a helper through PATH. Require complete real scan counts, apparent bytes, raw UTF-16 root preservation, exit code zero and zero active processes in the original Job. A second case rejects the original checkpoint after actual birth and requires the original Interrupted error plus real Job cleanup. These component tests do not grant image execution trust or replace CLI/MCP Runtime acceptance. Missing artifacts, build failure, zero tests or missing markers fail the gate.

## Observed loader GREEN at 84f284c

Run 37490308655 jobs 112361531819 (stable) and 112361532077 (1.97) each actually executed both loader cases: 2 passed, 0 failed, 0 ignored. Original artifact logs confirm System32 loading in both modes, unsigned DLL loading in Null and actual error 577 rejection in WorkerControl. Raw logs and exact source fingerprints are archived in windows_loader_native_green_84f. Both parent jobs later failed because the common protocol-driver build step used a Bash heredoc under PowerShell. Specify Bash for that exact step; retain its Cargo artifact identity and original assertions. Windows Kotlin and packaged runtime still return Unsupported, so no parent production gate is closed.

Paired Linux release run 37487309315 at f228b10 is separately archived with all 69 receipt artifact hashes and four source manifests verified. It records two rounds of 20k scans at 3.275/3.291s, 200k at 34.556/34.711s and 300-deep at 0.142/0.143s. These measurements do not represent 84f284c or grant all-platform acceptance.

## Current worker native GREEN at 57d6dd3

Run 37491505046 jobs 112365930087 (stable) and 112365930818 (1.97) each executed both actual worker cases: 2 passed, 0 failed, 0 ignored. The unpaired-surrogate root was scanned with complete counts/bytes, original native Job reached zero and normal exit was zero. Postbirth original Interrupted checkpoint and actual cleanup passed. Cargo output contains one current worker artifact and successful build; original receipts bind its size/SHA256 to this checkout. Raw evidence is retained in windows_worker_native_green_57d. The Bash driver-fixture step is now executed successfully. Stable parent still failed because CLI/HTTP index returned Unsupported exit 6. These results close the protocol component gate, not image admission, Runtime, FFI or packaged product acceptance. Add a five-minute CI step timeout for future protocol qualification; a CI timeout is failure, not proof of a finite product cleanup deadline.

## Borrowed process image-name binding API

Add a trusted local, read-only ScanWorkerHost API taking the caller's borrowed process handle and original deadline/checkpoint. Validate the retained original image/parent leases before and after QueryFullProcessImageNameW. Its bounded raw UTF-16 Win32 name must match the retained original name's normalized drive/components; then revalidate the original handle identity/version through the original parent chain. A byte-identical file at another name is not the original image. Query errors retain the original OS error; expiry/revocation must precede the query and poisoned/busy host state must not extend the original deadline.

This API verifies the reported process image name against retained material only. It does not prove mapped image bytes, eliminate pre-existing writable mappings, grant execution, resume the process or enable Runtime. Native tests inspect an actual suspended child, retain the original third-checkpoint error and clean the original Job before assertions. Require matching-image positive control, equal-bytes foreign-name conflict and original expired/revoked checks. Official API contract: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew . The full execution-permit and lease-retention design remains open.
