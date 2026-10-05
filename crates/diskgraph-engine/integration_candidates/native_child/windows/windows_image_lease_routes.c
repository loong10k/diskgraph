#include "windows_image_lease_probe.h"
#include <winioctl.h>
#include <string.h>

/* 仅自有临时目录的 mount-point reparse；不启用权限或修改全局设置。 */
static int create_junction(ImageLeaseProbe *p, const wchar_t *link, const wchar_t *target) {
    struct JunctionBuffer {
        ULONG tag;
        USHORT data_bytes;
        USHORT reserved;
        USHORT substitute_offset;
        USHORT substitute_bytes;
        USHORT print_offset;
        USHORT print_bytes;
        wchar_t paths[PROBE_PATH_CAP];
    } buffer;
    wchar_t substitute[PROBE_PATH_CAP];
    const wchar_t *ordinary = target;
    SIZE_T substitute_chars, print_chars;
    HANDLE directory;
    DWORD returned = 0, error;
    if (wcsncmp(ordinary, L"\\\\?\\", 4) == 0) ordinary += 4;
    if (ordinary[0] == L'\\' || swprintf_s(substitute, PROBE_PATH_CAP, L"\\?" L"?\\%ls", ordinary) < 0)
        return probe_fail(p, "junction_local_path_qualification", ERROR_INVALID_NAME);
    substitute_chars = wcslen(substitute);
    print_chars = wcslen(ordinary);
    if (substitute_chars + print_chars + 2 > PROBE_PATH_CAP)
        return probe_fail(p, "junction_path_bound", ERROR_BUFFER_OVERFLOW);
    ZeroMemory(&buffer, sizeof(buffer));
    buffer.tag = IO_REPARSE_TAG_MOUNT_POINT;
    buffer.substitute_bytes = (USHORT)(substitute_chars * sizeof(wchar_t));
    buffer.print_offset = (USHORT)((substitute_chars + 1) * sizeof(wchar_t));
    buffer.print_bytes = (USHORT)(print_chars * sizeof(wchar_t));
    buffer.data_bytes = (USHORT)(8 + (substitute_chars + print_chars + 2) * sizeof(wchar_t));
    memcpy(buffer.paths, substitute, (substitute_chars + 1) * sizeof(wchar_t));
    memcpy(buffer.paths + substitute_chars + 1, ordinary, (print_chars + 1) * sizeof(wchar_t));
    directory = CreateFileW(link, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                            NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, NULL);
    if (directory == INVALID_HANDLE_VALUE) {
        error = GetLastError(); return probe_fail(p, "junction_directory_open", error);
    }
    if (!DeviceIoControl(directory, FSCTL_SET_REPARSE_POINT, &buffer,
                          (DWORD)(8 + buffer.data_bytes), NULL, 0, &returned, NULL)) {
        error = GetLastError();
        if (!CloseHandle(directory)) { DWORD cleanup = GetLastError(); if (!p->cleanup_error) p->cleanup_error = cleanup; }
        return probe_fail(p, "junction_set_actual_reparse", error);
    }
    if (!CloseHandle(directory)) {
        error = GetLastError(); return probe_fail(p, "junction_directory_close", error);
    }
    return probe_check(p);
}

static int remove_junction_data(ImageLeaseProbe *p, const wchar_t *link) {
    struct ReparseHeader { ULONG tag; USHORT bytes; USHORT reserved; } header;
    HANDLE directory;
    DWORD returned = 0, error;
    ZeroMemory(&header, sizeof(header));
    header.tag = IO_REPARSE_TAG_MOUNT_POINT;
    directory = CreateFileW(link, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                            NULL, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, NULL);
    if (directory == INVALID_HANDLE_VALUE) {
        error = GetLastError(); return probe_fail(p, "junction_retarget_directory", error);
    }
    if (!DeviceIoControl(directory, FSCTL_DELETE_REPARSE_POINT, &header, sizeof(header),
                          NULL, 0, &returned, NULL)) {
        error = GetLastError();
        if (!CloseHandle(directory)) { DWORD cleanup = GetLastError(); if (!p->cleanup_error) p->cleanup_error = cleanup; }
        return probe_fail(p, "junction_remove_actual_reparse", error);
    }
    if (!CloseHandle(directory)) {
        error = GetLastError(); return probe_fail(p, "junction_retarget_close", error);
    }
    return probe_check(p);
}

static int verify_route_objects(ImageLeaseProbe *p, const wchar_t *route, BOOL changed, BOOL check_lease) {
    FILE_ID_INFO original, current, leased = {0};
    HANDLE reopened;
    DWORD error;
    reopened = CreateFileW(route, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                           NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (reopened == INVALID_HANDLE_VALUE) {
        error = GetLastError(); return probe_fail(p, "rebound_route_open", error);
    }
    if (!GetFileInformationByHandleEx(p->held, FileIdInfo, &original, sizeof(original)) ||
        (check_lease && !GetFileInformationByHandleEx(p->lease, FileIdInfo, &leased, sizeof(leased))) ||
        !GetFileInformationByHandleEx(reopened, FileIdInfo, &current, sizeof(current))) {
        error = GetLastError();
        if (!CloseHandle(reopened)) { DWORD cleanup = GetLastError(); if (!p->cleanup_error) p->cleanup_error = cleanup; }
        return probe_fail(p, "rebound_actual_full_file_id", error);
    }
    if (!CloseHandle(reopened)) {
        error = GetLastError(); return probe_fail(p, "rebound_route_close", error);
    }
    if (check_lease) {
        p->same_file = original.VolumeSerialNumber == leased.VolumeSerialNumber &&
                       memcmp(original.FileId.Identifier, leased.FileId.Identifier, 16) == 0;
    }
    p->route_changed = original.VolumeSerialNumber != current.VolumeSerialNumber ||
                        memcmp(original.FileId.Identifier, current.FileId.Identifier, 16) != 0;
    return ((!check_lease || p->same_file) && p->route_changed == changed) ||
           probe_fail(p, "route_identity_not_expected", ERROR_INVALID_DATA);
}

/* 仅资格候选：从 held 对象取得本地 NT 卷路径，不沿原 junction 或驱动器字母。 */
static int capture_kernel_route(ImageLeaseProbe *p, wchar_t *route) {
    wchar_t native[PROBE_PATH_CAP];
    const wchar_t *prefix = L"\\Device\\HarddiskVolume";
    const wchar_t *suffix;
    DWORD length, error;
    if (!probe_check(p)) return 0;
    length = GetFinalPathNameByHandleW(p->held, native, PROBE_PATH_CAP,
                                      FILE_NAME_NORMALIZED | VOLUME_NAME_NT);
    if (length == 0) {
        error = GetLastError(); return probe_fail(p, "kernel_route_query", error);
    }
    if (length >= PROBE_PATH_CAP || wcsncmp(native, prefix, wcslen(prefix)) != 0)
        return probe_fail(p, "kernel_route_local_volume_bound", ERROR_INVALID_NAME);
    suffix = native + wcslen(prefix);
    if (*suffix < L'0' || *suffix > L'9')
        return probe_fail(p, "kernel_route_volume_identifier", ERROR_INVALID_NAME);
    while (*suffix >= L'0' && *suffix <= L'9') ++suffix;
    if (*suffix != L'\\' || swprintf_s(route, PROBE_PATH_CAP, L"\\\\?\\GLOBALROOT%ls", native) < 0)
        return probe_fail(p, "kernel_route_prefix_bound", ERROR_BUFFER_OVERFLOW);
    return probe_check(p);
}

int probe_route_cases(ImageLeaseProbe *p, unsigned int case_id) {
    wchar_t container[PROBE_PATH_CAP], parked[PROBE_PATH_CAP], other[PROBE_PATH_CAP];
    wchar_t route[PROBE_PATH_CAP], other_image[PROBE_PATH_CAP], original_image[PROBE_PATH_CAP];
    DWORD error;
    wchar_t kernel_route[PROBE_PATH_CAP];
    if (!probe_path(container, p->root, L"container") || !probe_path(parked, p->root, L"parked") ||
        !probe_path(other, p->root, L"other") || !probe_path(route, container, L"image.exe") ||
        !probe_path(original_image, parked, L"image.exe") ||
        !probe_path(other_image, other, L"image.exe"))
        return probe_fail(p, "route_paths_bound", ERROR_BUFFER_OVERFLOW);
    if (!CreateDirectoryW(container, NULL)) {
        error = GetLastError(); return probe_fail(p, "original_container_qualification", error);
    }
    if (case_id >= 7) {
        if (!CreateDirectoryW(parked, NULL) || !CopyFileW(p->image_a, original_image, TRUE) ||
            !CreateDirectoryW(other, NULL) || !CopyFileW(p->image_b, other_image, TRUE)) {
            error = GetLastError(); return probe_fail(p, "junction_original_target_qualification", error);
        }
        if (!create_junction(p, container, parked) || !probe_execute(p, route, 'A')) return 0;
    } else if (!CopyFileW(p->image_a, route, TRUE)) {
        error = GetLastError(); return probe_fail(p, "original_route_copy", error);
    }
    if (wcscpy_s(p->image_a, PROBE_PATH_CAP, route) != 0)
        return probe_fail(p, "route_copy_bound", ERROR_BUFFER_OVERFLOW);
    if (!probe_open_held(p)) return 0;
    if (case_id == 6) {
        /* 同一非空目录、原 shareR/W/D held：先证明精确 rename 及还原可执行。 */
        if (!MoveFileExW(container, parked, 0)) {
            error = GetLastError(); return probe_fail(p, "ancestor_prelease_rename", error);
        }
        if (!MoveFileExW(parked, container, 0)) {
            error = GetLastError(); return probe_fail(p, "ancestor_prelease_restore", error);
        }
        if (!verify_route_objects(p, route, FALSE, FALSE) || !probe_execute(p, route, 'A')) return 0;
        p->ancestor_prelease_qualified = TRUE;
    }
    if (!probe_acquire_lease(p)) return probe_fail(p, "route_leaf_lease_qualification", p->lease_error);
    if (case_id == 8) {
        if (!capture_kernel_route(p, kernel_route) ||
            !verify_route_objects(p, kernel_route, FALSE, TRUE) ||
            !probe_execute(p, kernel_route, 'A')) return 0;
        p->kernel_route_qualified = TRUE;
    }
    if (case_id >= 7) {
        /* 此案独立改变同一个 junction 的数据，不依赖祖先 rename 是否被拒。 */
        if (!remove_junction_data(p, container)) return 0;
        if (!create_junction(p, container, other)) return 0;
    } else {
        if (!MoveFileExW(container, parked, 0)) {
            /* 普通 rename 被拒也须真实前后正控，不能将能力不足洗成保护成功。 */
            p->ancestor_rename_error = GetLastError();
            if (p->ancestor_rename_error != ERROR_ACCESS_DENIED &&
                p->ancestor_rename_error != ERROR_SHARING_VIOLATION)
                return probe_fail(p, "ancestor_rename_unexpected_error", p->ancestor_rename_error);
            if (!verify_route_objects(p, route, FALSE, TRUE) || !probe_execute(p, route, 'A')) return 0;
            p->route_unchanged_under_lease = TRUE;
            p->loaded_a_under_lease = TRUE;
            if (!CloseHandle(p->lease)) {
                error = GetLastError(); return probe_fail(p, "ancestor_lease_release", error);
            }
            p->lease = INVALID_HANDLE_VALUE;
            if (!MoveFileExW(container, parked, 0)) {
                error = GetLastError(); return probe_fail(p, "ancestor_released_rename", error);
            }
            p->released_rename_succeeded = TRUE;
        }
        if (!CreateDirectoryW(container, NULL) || !CopyFileW(p->image_b, route, TRUE)) {
            error = GetLastError(); return probe_fail(p, "replacement_b_qualification", error);
        }
    }
    if (!verify_route_objects(p, route, TRUE, p->lease != INVALID_HANDLE_VALUE) ||
        !probe_execute(p, route, 'B')) return 0;
    if (case_id == 8) {
        p->loaded_b_via_original_route = TRUE;
        if (!verify_route_objects(p, kernel_route, FALSE, TRUE) ||
            !probe_execute(p, kernel_route, 'A')) return 0;
        p->kernel_route_unchanged_after_rebind = TRUE;
        p->loaded_a_via_kernel_route = TRUE;
        p->classification = "resolved_kernel_route_survived_junction";
        p->stage = "original_route_b_resolved_route_a";
        return probe_check(p);
    }
    if (case_id == 6 && p->ancestor_rename_error != ERROR_SUCCESS) {
        p->loaded_b_after_release = TRUE;
        p->classification = "ancestor_rename_blocked_by_leaf_lease";
        p->stage = "ordinary_rename_denied_then_released_loaded_b";
        return probe_check(p);
    }
    /* 这证明 leaf-only 缺口；它绝不是安全能力或整条执行 lease 的通过。 */
    p->classification = "characterized_gap";
    p->stage = case_id == 7 ? "leaf_lease_junction_loaded_b" : "leaf_lease_ancestor_loaded_b";
    return probe_check(p);
}
