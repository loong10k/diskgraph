#ifndef DISKGRAPH_WINDOWS_IMAGE_LEASE_PROBE_H
#define DISKGRAPH_WINDOWS_IMAGE_LEASE_PROBE_H
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>
#include <wchar.h>

#define IMAGE_MARKER "DISKGRAPH_IMAGE_A_LEASE_MARKER"
#define IMAGE_MARKER_SIZE (sizeof(IMAGE_MARKER) - 1)
#define PROBE_PATH_CAP 4096
#define PROBE_IMAGE_CAP (128ULL * 1024ULL * 1024ULL)

/* 一个请求的实际 API 见证；不输出路径、File ID、原生内容或 HANDLE。 */
typedef struct ImageLeaseProbe {
    ULONGLONG deadline;
    ULONGLONG started;
    wchar_t root[PROBE_PATH_CAP];
    wchar_t image_a[PROBE_PATH_CAP];
    wchar_t image_b[PROBE_PATH_CAP];
    wchar_t output[PROBE_PATH_CAP];
    HANDLE held;
    HANDLE lease;
    HANDLE writer;
    HANDLE mapping;
    HANDLE image_section;
    unsigned char *view;
    SIZE_T view_bytes;
    SIZE_T marker_offset;
    const char *stage;
    const char *classification;
    DWORD win32_error;
    DWORD cleanup_error;
    DWORD exception_code;
    DWORD write_open_error;
    DWORD delete_open_error;
    DWORD replace_error;
    DWORD lease_error;
    DWORD image_error;
    DWORD ancestor_rename_error;
    DWORD ancestor_original_held_error;
    unsigned int launches;
    unsigned int actual_waits;
    unsigned int empty_jobs;
    BOOL same_file;
    BOOL writer_closed;
    BOOL mapping_handle_closed;
    BOOL view_changed;
    BOOL route_changed;
    BOOL ancestor_no_child_handles_qualified;
    BOOL route_unchanged_held_only;
    BOOL loaded_a_held_only;
    BOOL ancestor_lease_closed;
    BOOL ancestor_held_closed;
    BOOL route_unchanged_under_lease;
    BOOL loaded_a_under_lease;
    BOOL released_rename_succeeded;
    BOOL loaded_b_after_release;
    BOOL posix_api_qualified;
    BOOL kernel_route_qualified;
    BOOL loaded_b_via_original_route;
    BOOL kernel_route_unchanged_after_rebind;
    BOOL loaded_a_via_kernel_route;
    char loaded_marker;
} ImageLeaseProbe;

int probe_fail(ImageLeaseProbe *p, const char *stage, DWORD error);
int probe_check(ImageLeaseProbe *p);
int probe_path(wchar_t *out, const wchar_t *root, const wchar_t *leaf);
int probe_prepare(ImageLeaseProbe *p, const wchar_t *root,
                  const wchar_t *source_a, const wchar_t *source_b);
int probe_execute(ImageLeaseProbe *p, const wchar_t *image, char expected);
int probe_open_held(ImageLeaseProbe *p);
int probe_acquire_lease(ImageLeaseProbe *p);
int probe_map_writer(ImageLeaseProbe *p);
int probe_mutate_view(ImageLeaseProbe *p);
void probe_cleanup(ImageLeaseProbe *p);
int probe_basic_cases(ImageLeaseProbe *p, unsigned int case_id);
int probe_mapping_cases(ImageLeaseProbe *p, unsigned int case_id);
int probe_route_cases(ImageLeaseProbe *p, unsigned int case_id);
int probe_posix_case(ImageLeaseProbe *p);
#endif
