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

static int verify_rebound_objects(ImageLeaseProbe *p, const wchar_t *route) {
    FILE_ID_INFO original, current, leased;
    HANDLE reopened;
    DWORD error;
    reopened = CreateFileW(route, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                           NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (reopened == INVALID_HANDLE_VALUE) {
        error = GetLastError(); return probe_fail(p, "rebound_route_open", error);
    }
    if (!GetFileInformationByHandleEx(p->held, FileIdInfo, &original, sizeof(original)) ||
        !GetFileInformationByHandleEx(p->lease, FileIdInfo, &leased, sizeof(leased)) ||
        !GetFileInformationByHandleEx(reopened, FileIdInfo, &current, sizeof(current))) {
        error = GetLastError();
        if (!CloseHandle(reopened)) { DWORD cleanup = GetLastError(); if (!p->cleanup_error) p->cleanup_error = cleanup; }
        return probe_fail(p, "rebound_actual_full_file_id", error);
    }
    if (!CloseHandle(reopened)) {
        error = GetLastError(); return probe_fail(p, "rebound_route_close", error);
    }
    p->same_file = original.VolumeSerialNumber == leased.VolumeSerialNumber &&
                   memcmp(original.FileId.Identifier, leased.FileId.Identifier, 16) == 0;
    p->route_changed = original.VolumeSerialNumber != current.VolumeSerialNumber ||
                        memcmp(original.FileId.Identifier, current.FileId.Identifier, 16) != 0;
    return (p->same_file && p->route_changed) || probe_fail(p, "route_not_actually_rebound", ERROR_INVALID_DATA);
}

int probe_route_cases(ImageLeaseProbe *p, unsigned int case_id) {
    wchar_t container[PROBE_PATH_CAP], parked[PROBE_PATH_CAP], other[PROBE_PATH_CAP];
    wchar_t route[PROBE_PATH_CAP], other_image[PROBE_PATH_CAP], original_image[PROBE_PATH_CAP];
    DWORD error;
    if (!probe_path(container, p->root, L"container") || !probe_path(parked, p->root, L"parked") ||
        !probe_path(other, p->root, L"other") || !probe_path(route, container, L"image.exe") ||
        !probe_path(original_image, parked, L"image.exe") ||
        !probe_path(other_image, other, L"image.exe"))
        return probe_fail(p, "route_paths_bound", ERROR_BUFFER_OVERFLOW);
    if (!CreateDirectoryW(container, NULL)) {
        error = GetLastError(); return probe_fail(p, "original_container_qualification", error);
    }
    if (case_id == 7) {
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
    if (!probe_acquire_lease(p)) return probe_fail(p, "route_leaf_lease_qualification", p->lease_error);
    if (case_id == 7) {
        /* 此案独立改变同一个 junction 的数据，不依赖祖先 rename 是否被拒。 */
        if (!remove_junction_data(p, container)) return 0;
        if (!create_junction(p, container, other)) return 0;
    } else {
        if (!MoveFileExW(container, parked, 0)) {
            error = GetLastError(); return probe_fail(p, "ancestor_rename_qualification", error);
        }
        if (!CreateDirectoryW(container, NULL) || !CopyFileW(p->image_b, route, TRUE)) {
            error = GetLastError(); return probe_fail(p, "replacement_b_qualification", error);
        }
    }
    if (!verify_rebound_objects(p, route) || !probe_execute(p, route, 'B')) return 0;
    /* 这证明 leaf-only 缺口；它绝不是安全能力或整条执行 lease 的通过。 */
    p->classification = "characterized_gap";
    p->stage = case_id == 7 ? "leaf_lease_junction_loaded_b" : "leaf_lease_ancestor_loaded_b";
    return probe_check(p);
}
