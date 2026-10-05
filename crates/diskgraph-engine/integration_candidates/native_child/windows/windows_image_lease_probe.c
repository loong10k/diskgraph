#include "windows_image_lease_probe.h"
#include <stdlib.h>

/* 每案是独立原生探针，不实现或挂载 Windows 生产启动器。 */
int wmain(int argc, wchar_t **argv) {
    ImageLeaseProbe p;
    unsigned int case_id;
    ULONGLONG original_deadline;
    int ok = 0;
    ZeroMemory(&p, sizeof(p));
    p.held = p.lease = p.writer = p.mapping = p.image_section = INVALID_HANDLE_VALUE;
    p.stage = "argument_qualification";
    p.classification = "qualification_failure";
    if (argc != 6) {
        return 93;
    }
    case_id = (unsigned int)wcstoul(argv[1], NULL, 10);
    original_deadline = wcstoull(argv[5], NULL, 10);
    p.started = GetTickCount64();
    if (case_id < 1 || case_id > 7 || original_deadline <= p.started ||
        original_deadline - p.started > 20000) {
        return 93;
    }
    p.deadline = original_deadline;
    if (probe_prepare(&p, argv[2], argv[3], argv[4])) {
        p.loaded_marker = 0;
        if (case_id <= 3) {
            ok = probe_basic_cases(&p, case_id);
        } else if (case_id <= 5) {
            ok = probe_mapping_cases(&p, case_id);
        } else {
            ok = probe_route_cases(&p, case_id);
        }
    }
    probe_cleanup(&p);
    if (p.cleanup_error != ERROR_SUCCESS) {
        ok = 0;
    }
    printf("{\"case\":%u,\"classification\":\"%s\",\"stage\":\"%s\","
           "\"win32_error\":%lu,\"cleanup_error\":%lu,\"exception_code\":%lu,"
           "\"write_open_error\":%lu,\"delete_open_error\":%lu,\"replace_error\":%lu,"
           "\"lease_error\":%lu,\"image_error\":%lu,\"same_file\":%s,"
           "\"writer_closed\":%s,\"mapping_handle_closed\":%s,\"view_changed\":%s,"
           "\"route_changed\":%s,\"ancestor_rename_error\":%lu,"
           "\"ancestor_prelease_qualified\":%s,\"route_unchanged_under_lease\":%s,"
           "\"loaded_a_under_lease\":%s,\"released_rename_succeeded\":%s,"
           "\"loaded_b_after_release\":%s,\"loaded_marker\":\"%c\",\"launches\":%u,"
           "\"actual_waits\":%u,\"empty_jobs\":%u,\"elapsed_ms\":%llu}\n",
           case_id, p.classification, p.stage, p.win32_error, p.cleanup_error,
           p.exception_code, p.write_open_error, p.delete_open_error, p.replace_error,
           p.lease_error, p.image_error, p.same_file ? "true" : "false",
           p.writer_closed ? "true" : "false", p.mapping_handle_closed ? "true" : "false",
           p.view_changed ? "true" : "false", p.route_changed ? "true" : "false",
           p.ancestor_rename_error, p.ancestor_prelease_qualified ? "true" : "false",
           p.route_unchanged_under_lease ? "true" : "false", p.loaded_a_under_lease ? "true" : "false",
           p.released_rename_succeeded ? "true" : "false", p.loaded_b_after_release ? "true" : "false",
           p.loaded_marker ? p.loaded_marker : '-', p.launches, p.actual_waits,
           p.empty_jobs, GetTickCount64() - p.started);
    fflush(stdout);
    return ok ? 0 : 1;
}
