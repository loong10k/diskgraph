#include "windows_image_lease_probe.h"
#include <stddef.h>
#include <string.h>

/* 同一个公开 FileRenameInfoEx、相同 flags=REPLACE|POSIX，禁止换 API 做释放后正控。 */
static BOOL replace_posix(HANDLE source, const wchar_t *target, DWORD *error) {
    union RenameBuffer {
        FILE_RENAME_INFO alignment;
        unsigned char bytes[sizeof(FILE_RENAME_INFO) + PROBE_PATH_CAP * sizeof(wchar_t)];
    } storage;
    FILE_RENAME_INFO *info = (FILE_RENAME_INFO *)storage.bytes;
    SIZE_T chars = wcslen(target), bytes;
    if (chars == 0 || chars >= PROBE_PATH_CAP) { *error = ERROR_INVALID_NAME; return FALSE; }
    bytes = chars * sizeof(wchar_t);
    ZeroMemory(&storage, sizeof(storage));
    info->Flags = 0x00000001UL | 0x00000002UL;
    info->RootDirectory = NULL;
    info->FileNameLength = (DWORD)bytes;
    memcpy(info->FileName, target, bytes);
    if (!SetFileInformationByHandle(source, FileRenameInfoEx, info,
                                    (DWORD)(sizeof(FILE_RENAME_INFO) + bytes))) {
        *error = GetLastError(); return FALSE;
    }
    *error = ERROR_SUCCESS;
    return TRUE;
}

static int close_owned(ImageLeaseProbe *p, HANDLE *handle, const char *stage) {
    DWORD error;
    if (!CloseHandle(*handle)) { error = GetLastError(); return probe_fail(p, stage, error); }
    *handle = INVALID_HANDLE_VALUE;
    return probe_check(p);
}

static int open_source(ImageLeaseProbe *p, const wchar_t *source) {
    p->writer = CreateFileW(source, DELETE | FILE_READ_ATTRIBUTES,
                            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                            NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (p->writer == INVALID_HANDLE_VALUE) {
        DWORD error = GetLastError(); return probe_fail(p, "posix_source_delete_access", error);
    }
    return probe_check(p);
}

/* 每次原 held 和 route 都实际查询完整128位ID；临时route句柄立即关闭。 */
static int compare_route(ImageLeaseProbe *p, const wchar_t *route, BOOL changed, BOOL lease) {
    FILE_ID_INFO original = {0}, current = {0}, leased = {0};
    HANDLE reopened;
    DWORD error;
    reopened = CreateFileW(route, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                           NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (reopened == INVALID_HANDLE_VALUE) {
        error = GetLastError(); return probe_fail(p, "posix_route_reopen", error);
    }
    if (!GetFileInformationByHandleEx(p->held, FileIdInfo, &original, sizeof(original)) ||
        !GetFileInformationByHandleEx(reopened, FileIdInfo, &current, sizeof(current)) ||
        (lease && !GetFileInformationByHandleEx(p->lease, FileIdInfo, &leased, sizeof(leased)))) {
        error = GetLastError();
        if (!CloseHandle(reopened) && !p->cleanup_error) p->cleanup_error = GetLastError();
        return probe_fail(p, "posix_full_identity", error);
    }
    if (!CloseHandle(reopened)) {
        error = GetLastError(); return probe_fail(p, "posix_route_close", error);
    }
    if (lease) p->same_file = original.VolumeSerialNumber == leased.VolumeSerialNumber &&
                            memcmp(original.FileId.Identifier, leased.FileId.Identifier, 16) == 0;
    p->route_changed = original.VolumeSerialNumber != current.VolumeSerialNumber ||
                       memcmp(original.FileId.Identifier, current.FileId.Identifier, 16) != 0;
    return ((!lease || p->same_file) && p->route_changed == changed) ||
           probe_fail(p, "posix_identity_not_expected", ERROR_INVALID_DATA);
}

int probe_posix_case(ImageLeaseProbe *p) {
    wchar_t positive[PROBE_PATH_CAP], source[PROBE_PATH_CAP], attack[PROBE_PATH_CAP];
    DWORD error = ERROR_SUCCESS;
    BOOL replaced;
    if (!probe_path(positive, p->root, L"posix_positive.exe") ||
        !probe_path(source, p->root, L"posix_positive_source.exe") ||
        !probe_path(attack, p->root, L"posix_attack_source.exe"))
        return probe_fail(p, "posix_path_bounds", ERROR_BUFFER_OVERFLOW);
    if (!CopyFileW(p->image_a, positive, TRUE) || !CopyFileW(p->image_b, source, TRUE) ||
        !CopyFileW(p->image_b, attack, TRUE)) {
        error = GetLastError(); return probe_fail(p, "posix_fixture_copies", error);
    }
    /* 原读句柄shareR/W/D保持打开：先验证POSIX换名真实可用，并实际执行B。 */
    p->held = CreateFileW(positive, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                          NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (p->held == INVALID_HANDLE_VALUE) {
        error = GetLastError(); return probe_fail(p, "posix_positive_held", error);
    }
    if (!open_source(p, source)) return 0;
    if (!replace_posix(p->writer, positive, &error))
        return probe_fail(p, "posix_same_api_positive", error);
    if (!close_owned(p, &p->writer, "posix_positive_source_close") ||
        !compare_route(p, positive, TRUE, FALSE) || !probe_execute(p, positive, 'B')) return 0;
    p->posix_api_qualified = TRUE;
    if (!close_owned(p, &p->held, "posix_positive_held_close")) return 0;
    if (!probe_open_held(p)) return 0;
    if (!probe_acquire_lease(p)) return probe_fail(p, "posix_leaf_lease", p->lease_error);
    if (!compare_route(p, p->image_a, FALSE, TRUE) || !open_source(p, attack)) return 0;
    if (!probe_check(p)) return 0;
    replaced = replace_posix(p->writer, p->image_a, &p->replace_error);
    if (!replaced) {
        if (p->replace_error != ERROR_ACCESS_DENIED && p->replace_error != ERROR_SHARING_VIOLATION)
            return probe_fail(p, "posix_unexpected_denial", p->replace_error);
        if (!compare_route(p, p->image_a, FALSE, TRUE) || !probe_execute(p, p->image_a, 'A')) return 0;
        p->route_unchanged_under_lease = TRUE;
        p->loaded_a_under_lease = TRUE;
        if (!close_owned(p, &p->lease, "posix_lease_release")) return 0;
        p->ancestor_lease_closed = TRUE;
        /* 同一source句柄、同一target、同一flags，保留原held而仅释放lease。 */
        if (!replace_posix(p->writer, p->image_a, &error))
            return probe_fail(p, "posix_released_same_api_positive", error);
        p->released_rename_succeeded = TRUE;
    }
    if (!close_owned(p, &p->writer, "posix_attack_source_close") ||
        !compare_route(p, p->image_a, TRUE, replaced) || !probe_execute(p, p->image_a, 'B')) return 0;
    p->loaded_b_after_release = !replaced;
    p->classification = replaced ? "posix_leaf_lease_gap" : "posix_leaf_lease_rejected_replacement";
    p->stage = replaced ? "posix_replaced_actual_b_with_held_a" : "posix_denied_then_same_api_released_actual_b";
    return probe_check(p);
}
