# macOS installation checks by execution phase

The source-e7f16ff native Intel regression in run 37485522359/job 112345124353 returned PermissionDenied at real token expiry before the native observation hook (3.12 seconds). The original token validity remains three seconds. Do not change it to conceal startup cost.

Before birth, prepare_macos_installation acquires the original installation update lock, reads and verifies active settings, performs full held image/namespace/digest/loader validation, and reads active settings again. MacosSpawnPermit and MacosNativeLauncher retain the same update lock through the native birth call. All steps keep the original request authorization/fencing/cancellation/deadline checkpoint. Avoid wrapping those per-block checks with another complete epoch/configuration read.

After actual birth, the Runtime installs its epoch-check wrapper for protocol polling. Runtime failure/panic cleanup and retained original owner semantics remain unchanged. Staging and publication authorization gates are unchanged.

Acceptance requires the original native Intel three-second observation-expiry case to execute and pass, plus existing installation update, revocation, image/namespace integrity and driver regressions. Six local installation lease cases, twelve real driver cases and six source-layout cases passed; these do not prove native product acceptance. No parent task is closed.
