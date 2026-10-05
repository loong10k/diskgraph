#include "windows_image_lease_probe.h"
#include <string.h>

static int deny_open(ImageLeaseProbe *p, DWORD access, DWORD *actual_error) {
    HANDLE attempt;
    attempt = CreateFileW(p->image_a, access,
                          FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                          NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (attempt == INVALID_HANDLE_VALUE) {
        *actual_error = GetLastError();
        return *actual_error == ERROR_SHARING_VIOLATION ||
               probe_fail(p, "leaf_open_unexpected_error", *actual_error);
    }
    if (!CloseHandle(attempt)) {
        DWORD error = GetLastError(); if (!p->cleanup_error) p->cleanup_error = error;
    }
    return probe_fail(p, "leaf_open_not_denied", ERROR_INVALID_DATA);
}

static int write_same_header_byte(ImageLeaseProbe *p, HANDLE writer) {
    const char mz = 'M';
    DWORD written = 0, error;
    if (!WriteFile(writer, &mz, 1, &written, NULL)) {
        error = GetLastError(); return probe_fail(p, "unleased_actual_write", error);
    }
    if (written != 1) return probe_fail(p, "unleased_write_zero", ERROR_WRITE_FAULT);
    if (!FlushFileBuffers(writer)) {
        error = GetLastError(); return probe_fail(p, "unleased_write_flush", error);
    }
    return probe_check(p);
}

int probe_basic_cases(ImageLeaseProbe *p, unsigned int case_id) {
    DWORD error;
    HANDLE writer;
    wchar_t movable[PROBE_PATH_CAP], moved[PROBE_PATH_CAP];
    wchar_t replaced[PROBE_PATH_CAP], replacing[PROBE_PATH_CAP];
    if (case_id == 1) {
        writer = CreateFileW(p->image_a, GENERIC_WRITE,
                             FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                             NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if (writer == INVALID_HANDLE_VALUE) {
            error = GetLastError(); return probe_fail(p, "unleased_writer_qualification", error);
        }
        if (!write_same_header_byte(p, writer)) {
            if (!CloseHandle(writer)) { DWORD cleanup = GetLastError(); if (!p->cleanup_error) p->cleanup_error = cleanup; }
            return 0;
        }
        if (!CloseHandle(writer)) { error = GetLastError(); return probe_fail(p, "unleased_writer_close", error); }
        if (!probe_path(movable, p->root, L"movable") || !probe_path(moved, p->root, L"moved"))
            return probe_fail(p, "unleased_route_bound", ERROR_BUFFER_OVERFLOW);
        if (!CreateDirectoryW(movable, NULL) || !MoveFileExW(movable, moved, 0)) {
            error = GetLastError(); return probe_fail(p, "unleased_rename_qualification", error);
        }
        if (!probe_path(replaced, p->root, L"replace_positive.exe") ||
            !probe_path(replacing, p->root, L"replace_source.exe"))
            return probe_fail(p, "unleased_replacement_bound", ERROR_BUFFER_OVERFLOW);
        if (!CopyFileW(p->image_a, replaced, TRUE) || !CopyFileW(p->image_b, replacing, TRUE) ||
            !MoveFileExW(replacing, replaced, MOVEFILE_REPLACE_EXISTING)) {
            error = GetLastError(); return probe_fail(p, "unleased_replacement_qualification", error);
        }
        if (!probe_execute(p, replaced, 'B')) return 0;
        /* 验证 SEC_IMAGE 在无攻击者映射时确实能准入同一个有效 PE。 */
        if (!probe_open_held(p)) return 0;
        p->image_section = CreateFileMappingW(p->held, NULL, PAGE_READONLY | SEC_IMAGE, 0, 0, NULL);
        if (!p->image_section) {
            error = GetLastError(); return probe_fail(p, "unleased_sec_image_qualification", error);
        }
        p->classification = "fixture_qualified";
        p->stage = "real_a_b_and_sec_image_qualified";
        return probe_check(p);
    }
    if (!probe_open_held(p)) return 0;
    if (case_id == 3) {
        p->writer = CreateFileW(p->image_a, GENERIC_READ | GENERIC_WRITE,
                                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                                NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if (p->writer == INVALID_HANDLE_VALUE) {
            error = GetLastError(); return probe_fail(p, "live_writer_qualification", error);
        }
        if (!write_same_header_byte(p, p->writer)) return 0;
        if (probe_acquire_lease(p) || p->lease_error != ERROR_SHARING_VIOLATION)
            return probe_fail(p, "live_writer_lease_not_denied", p->lease_error);
        if (!CloseHandle(p->writer)) {
            error = GetLastError(); return probe_fail(p, "live_writer_close", error);
        }
        p->writer = INVALID_HANDLE_VALUE;
        p->writer_closed = TRUE;
        /* 一个独立的释放后正控，不覆盖前一实际拒绝的 lease_error。 */
        if (!probe_acquire_lease(p)) return probe_fail(p, "released_writer_positive_control", p->lease_error);
        if (!probe_execute(p, p->image_a, 'A')) return 0;
        p->classification = "live_writer_rejected";
        p->stage = "live_writer_rejected_and_released_positive";
        return probe_check(p);
    }
    if (!probe_acquire_lease(p)) return probe_fail(p, "leaf_lease_qualification", p->lease_error);
    if (!deny_open(p, GENERIC_WRITE, &p->write_open_error) ||
        !deny_open(p, DELETE, &p->delete_open_error)) return 0;
    if (MoveFileExW(p->image_b, p->image_a, MOVEFILE_REPLACE_EXISTING))
        return probe_fail(p, "leaf_replacement_not_denied", ERROR_INVALID_DATA);
    p->replace_error = GetLastError();
    if (p->replace_error != ERROR_SHARING_VIOLATION && p->replace_error != ERROR_ACCESS_DENIED)
        return probe_fail(p, "leaf_replacement_unexpected_error", p->replace_error);
    if (!probe_execute(p, p->image_a, 'A')) return 0;
    p->classification = "leaf_write_delete_blocked";
    p->stage = "lease_held_through_actual_a_load";
    return probe_check(p);
}

int probe_map_writer(ImageLeaseProbe *p) {
    LARGE_INTEGER bytes;
    SIZE_T offset, occurrences = 0;
    DWORD error;
    p->writer = CreateFileW(p->image_a, GENERIC_READ | GENERIC_WRITE,
                            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                            NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (p->writer == INVALID_HANDLE_VALUE) {
        error = GetLastError(); return probe_fail(p, "writable_view_original_writer", error);
    }
    if (!GetFileSizeEx(p->writer, &bytes)) {
        error = GetLastError(); return probe_fail(p, "writable_view_size", error);
    }
    if (bytes.QuadPart <= (LONGLONG)IMAGE_MARKER_SIZE || (ULONGLONG)bytes.QuadPart > PROBE_IMAGE_CAP)
        return probe_fail(p, "writable_view_size_bound", ERROR_FILE_TOO_LARGE);
    p->view_bytes = (SIZE_T)bytes.QuadPart;
    p->mapping = CreateFileMappingW(p->writer, NULL, PAGE_READWRITE, 0, 0, NULL);
    if (!p->mapping) { error = GetLastError(); return probe_fail(p, "writable_mapping_create", error); }
    p->view = (unsigned char *)MapViewOfFile(p->mapping, FILE_MAP_WRITE, 0, 0, p->view_bytes);
    if (!p->view) { error = GetLastError(); return probe_fail(p, "writable_view_create", error); }
    for (offset = 0; offset <= p->view_bytes - IMAGE_MARKER_SIZE; ++offset) {
        if (offset % (64 * 1024) == 0 && !probe_check(p)) return 0;
        if (memcmp(p->view + offset, IMAGE_MARKER, IMAGE_MARKER_SIZE) == 0) {
            p->marker_offset = offset;
            ++occurrences;
        }
    }
    if (occurrences != 1) return probe_fail(p, "unique_actual_pe_marker", ERROR_INVALID_DATA);
    /* 写同一字节只验证 FILE_MAP_WRITE 真实可写，不改原 A 内容。 */
    __try {
        *((volatile unsigned char *)p->view + p->marker_offset + 16) = 'A';
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        p->exception_code = GetExceptionCode();
        return probe_fail(p, "writable_view_initial_store", ERROR_INVALID_DATA);
    }
    if (!CloseHandle(p->writer)) {
        error = GetLastError(); return probe_fail(p, "writable_original_close", error);
    }
    p->writer = INVALID_HANDLE_VALUE;
    p->writer_closed = TRUE;
    if (!CloseHandle(p->mapping)) {
        error = GetLastError(); return probe_fail(p, "mapping_handle_close", error);
    }
    p->mapping = INVALID_HANDLE_VALUE;
    p->mapping_handle_closed = TRUE;
    return probe_check(p);
}

int probe_mutate_view(ImageLeaseProbe *p) {
    __try {
        *((volatile unsigned char *)p->view + p->marker_offset + 16) = 'B';
        p->view_changed = p->view[p->marker_offset + 16] == 'B';
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        p->exception_code = GetExceptionCode();
        return 0;
    }
    if (!FlushViewOfFile(p->view, p->view_bytes)) {
        DWORD error = GetLastError(); return probe_fail(p, "actual_view_flush", error);
    }
    return p->view_changed;
}

int probe_mapping_cases(ImageLeaseProbe *p, unsigned int case_id) {
    HANDLE positive_section, positive_file;
    DWORD error;
    int mutated;
    if (!probe_open_held(p)) return 0;
    if (case_id == 5) {
        /* 在独立 B 上验证 SEC_IMAGE，避免预热受测 A 的 image section 干扰反控。 */
        positive_file = CreateFileW(p->image_b, GENERIC_READ,
                                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                                    NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if (positive_file == INVALID_HANDLE_VALUE) {
            error = GetLastError(); return probe_fail(p, "clean_sec_image_file", error);
        }
        positive_section = CreateFileMappingW(positive_file, NULL, PAGE_READONLY | SEC_IMAGE, 0, 0, NULL);
        if (!positive_section) {
            error = GetLastError();
            if (!CloseHandle(positive_file)) { DWORD cleanup = GetLastError(); if (!p->cleanup_error) p->cleanup_error = cleanup; }
            return probe_fail(p, "clean_sec_image_positive", error);
        }
        if (!CloseHandle(positive_section)) {
            error = GetLastError();
            if (!CloseHandle(positive_file)) { DWORD cleanup = GetLastError(); if (!p->cleanup_error) p->cleanup_error = cleanup; }
            return probe_fail(p, "clean_sec_image_close", error);
        }
        if (!CloseHandle(positive_file)) {
            error = GetLastError(); return probe_fail(p, "clean_sec_image_file_close", error);
        }
    }
    if (!probe_map_writer(p)) return 0;
    if (!probe_acquire_lease(p)) {
        if (p->win32_error || p->lease_error != ERROR_SHARING_VIOLATION)
            return probe_fail(p, "mapped_view_lease_unexpected_error", p->lease_error);
        p->classification = "mapped_writer_admission_rejected";
        p->stage = "reopen_rejected_existing_writable_view";
        return probe_check(p);
    }
    if (case_id == 5) {
        p->image_section = CreateFileMappingW(p->lease, NULL, PAGE_READONLY | SEC_IMAGE, 0, 0, NULL);
        if (!p->image_section) {
            p->image_error = GetLastError();
            if (p->image_error != ERROR_SHARING_VIOLATION && p->image_error != ERROR_USER_MAPPED_FILE &&
                p->image_error != ERROR_ACCESS_DENIED)
                return probe_fail(p, "mapped_sec_image_unexpected_error", p->image_error);
            p->classification = "mapped_writer_admission_rejected";
            p->stage = "sec_image_rejected_existing_writable_view";
            return probe_check(p);
        }
    }
    mutated = probe_mutate_view(p);
    if (p->view_changed) {
        /* 内容一旦可变即阻断；实际 loader 留 A 或拒加载都不能洗掉这个反例。 */
        p->classification = "blocked_mutable_image";
        if (mutated && !p->win32_error) (void)probe_execute(p, p->image_a, 'B');
        if (!p->win32_error) p->stage = "lease_acquired_but_existing_view_changed";
        return 0;
    }
    if (p->exception_code != EXCEPTION_ACCESS_VIOLATION)
        return probe_fail(p, "view_write_not_qualified", ERROR_INVALID_DATA);
    if (!probe_execute(p, p->image_a, 'A')) return 0;
    p->classification = "existing_view_write_blocked";
    p->stage = "lease_held_view_store_denied_actual_a";
    return probe_check(p);
}
