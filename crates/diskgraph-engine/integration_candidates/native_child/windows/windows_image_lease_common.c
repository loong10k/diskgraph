#include "windows_image_lease_probe.h"
#include <string.h>

int probe_fail(ImageLeaseProbe *p, const char *stage, DWORD error) {
    if (p->win32_error == ERROR_SUCCESS) {
        p->stage = stage;
        p->win32_error = error ? error : ERROR_INVALID_DATA;
    }
    return 0;
}

int probe_check(ImageLeaseProbe *p) {
    return GetTickCount64() < p->deadline || probe_fail(p, "original_deadline", ERROR_TIMEOUT);
}

int probe_path(wchar_t *out, const wchar_t *root, const wchar_t *leaf) {
    return swprintf_s(out, PROBE_PATH_CAP, L"%ls\\%ls", root, leaf) > 0;
}

static void close_local(ImageLeaseProbe *p, HANDLE handle) {
    if (handle != INVALID_HANDLE_VALUE && handle != NULL && !CloseHandle(handle)) {
        DWORD error = GetLastError();
        if (p->cleanup_error == ERROR_SUCCESS) p->cleanup_error = error;
    }
}

/* 成功只取真实 leader wait + ActiveProcesses=0；错误救援不充许可。 */
int probe_execute(ImageLeaseProbe *p, const wchar_t *image, char expected) {
    HANDLE job = INVALID_HANDLE_VALUE, output = INVALID_HANDLE_VALUE, input = INVALID_HANDLE_VALUE;
    PROCESS_INFORMATION process;
    STARTUPINFOEXW startup;
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits;
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION accounting;
    SECURITY_ATTRIBUTES security = {sizeof(SECURITY_ATTRIBUTES), NULL, TRUE};
    union AttributeStorage { void *alignment; unsigned char bytes[1024]; } attribute_storage;
    SIZE_T attribute_bytes = sizeof(attribute_storage.bytes);
    HANDLE inherited[2];
    wchar_t command[PROBE_PATH_CAP + 4];
    char marker[IMAGE_MARKER_SIZE + 2];
    DWORD read_bytes = 0, exit_code = 0, waited, error, resumed;
    ULONGLONG wait_started;
    LARGE_INTEGER zero;
    int attribute_initialized = 0, born = 0, assigned = 0, waited_actual = 0, ok = 0;
    ZeroMemory(&process, sizeof(process));
    ZeroMemory(&startup, sizeof(startup));
    ZeroMemory(&limits, sizeof(limits));
    zero.QuadPart = 0;
    if (!probe_check(p)) goto done;
    job = CreateJobObjectW(NULL, NULL);
    if (!job) { error = GetLastError(); probe_fail(p, "marker_job_create", error); goto done; }
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits, sizeof(limits))) {
        error = GetLastError(); probe_fail(p, "marker_job_limits", error); goto done;
    }
    output = CreateFileW(p->output, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ,
                         &security, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (output == INVALID_HANDLE_VALUE) { error = GetLastError(); probe_fail(p, "marker_output", error); goto done; }
    input = CreateFileW(L"NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE,
                        &security, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (input == INVALID_HANDLE_VALUE) { error = GetLastError(); probe_fail(p, "marker_input", error); goto done; }
    startup.lpAttributeList = (LPPROC_THREAD_ATTRIBUTE_LIST)attribute_storage.bytes;
    if (!InitializeProcThreadAttributeList(startup.lpAttributeList, 2, 0, &attribute_bytes)) {
        error = GetLastError(); probe_fail(p, "marker_attribute_init", error); goto done;
    }
    attribute_initialized = 1;
    inherited[0] = input;
    inherited[1] = output;
    if (!UpdateProcThreadAttribute(startup.lpAttributeList, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                                   inherited, sizeof(inherited), NULL, NULL)) {
        error = GetLastError(); probe_fail(p, "marker_allowlist", error); goto done;
    }
    /* 原子绑定私有 Job，探针在 CreateProcess 返回前被终止也不会留下悬挂 child。 */
    if (!UpdateProcThreadAttribute(startup.lpAttributeList, 0, PROC_THREAD_ATTRIBUTE_JOB_LIST,
                                   &job, sizeof(job), NULL, NULL)) {
        error = GetLastError(); probe_fail(p, "marker_atomic_job_list", error); goto done;
    }
    startup.StartupInfo.cb = sizeof(startup);
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = input;
    startup.StartupInfo.hStdOutput = output;
    startup.StartupInfo.hStdError = output;
    if (swprintf_s(command, PROBE_PATH_CAP + 4, L"\"%ls\"", image) < 0) {
        probe_fail(p, "marker_command_bound", ERROR_BUFFER_OVERFLOW); goto done;
    }
    if (!CreateProcessW(image, command, NULL, NULL, TRUE,
                        CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                        NULL, p->root, &startup.StartupInfo, &process)) {
        error = GetLastError(); probe_fail(p, "marker_create_process", error); goto done;
    }
    born = 1;
    ++p->launches;
    /* 只有成功的原子创建才进入此分支；不以出生后 Assign 修补窗口。 */
    assigned = 1;
    resumed = ResumeThread(process.hThread);
    if (resumed != 1) {
        error = resumed == (DWORD)-1 ? GetLastError() : ERROR_INVALID_DATA;
        probe_fail(p, "marker_resume", error); goto done;
    }
    wait_started = GetTickCount64();
    if (wait_started >= p->deadline) { probe_fail(p, "original_deadline", ERROR_TIMEOUT); goto done; }
    waited = WaitForSingleObject(process.hProcess, (DWORD)(p->deadline - wait_started));
    if (waited != WAIT_OBJECT_0) {
        error = waited == WAIT_FAILED ? GetLastError() : ERROR_TIMEOUT;
        probe_fail(p, "marker_actual_wait", error); goto done;
    }
    waited_actual = 1;
    ++p->actual_waits;
    if (!GetExitCodeProcess(process.hProcess, &exit_code)) {
        error = GetLastError(); probe_fail(p, "marker_exit_code", error); goto done;
    }
    for (;;) {
        if (!QueryInformationJobObject(job, JobObjectBasicAccountingInformation,
                                       &accounting, sizeof(accounting), NULL)) {
            error = GetLastError(); probe_fail(p, "marker_job_accounting", error); goto done;
        }
        if (accounting.ActiveProcesses == 0) break;
        if (!probe_check(p)) goto done;
        Sleep(1);
    }
    ++p->empty_jobs;
    if (!SetFilePointerEx(output, zero, NULL, FILE_BEGIN) ||
        !ReadFile(output, marker, sizeof(marker), &read_bytes, NULL)) {
        error = GetLastError(); probe_fail(p, "marker_actual_output", error); goto done;
    }
    if (exit_code != 0 || read_bytes != IMAGE_MARKER_SIZE + 1 ||
        memcmp(marker, IMAGE_MARKER, 16) != 0 || (marker[16] != 'A' && marker[16] != 'B') ||
        memcmp(marker + 17, IMAGE_MARKER + 17, IMAGE_MARKER_SIZE - 17) != 0 ||
        marker[IMAGE_MARKER_SIZE] != '\n') {
        probe_fail(p, "marker_actual_identity", ERROR_INVALID_DATA); goto done;
    }
    p->loaded_marker = marker[16];
    if (p->loaded_marker != expected) {
        probe_fail(p, "marker_unexpected_loaded_identity", ERROR_INVALID_DATA); goto done;
    }
    ok = probe_check(p);
done:
    if (born && !ok) {
        BOOL terminated = assigned ? TerminateJobObject(job, 92) : TerminateProcess(process.hProcess, 92);
        if (!terminated) { error = GetLastError(); if (!p->cleanup_error) p->cleanup_error = error; }
        if (!waited_actual && WaitForSingleObject(process.hProcess, 1000) != WAIT_OBJECT_0 && !p->cleanup_error)
            p->cleanup_error = ERROR_TIMEOUT;
    }
    if (attribute_initialized) DeleteProcThreadAttributeList(startup.lpAttributeList);
    close_local(p, process.hThread);
    close_local(p, process.hProcess);
    close_local(p, input);
    close_local(p, output);
    close_local(p, job);
    return ok;
}

int probe_prepare(ImageLeaseProbe *p, const wchar_t *root,
                  const wchar_t *source_a, const wchar_t *source_b) {
    DWORD error;
    if (wcscpy_s(p->root, PROBE_PATH_CAP, root) != 0 ||
        !probe_path(p->image_a, root, L"image_a.exe") ||
        !probe_path(p->image_b, root, L"image_b.exe") ||
        !probe_path(p->output, root, L"marker_output.txt"))
        return probe_fail(p, "path_bound", ERROR_BUFFER_OVERFLOW);
    if (!probe_check(p)) return 0;
    if (!CopyFileW(source_a, p->image_a, TRUE) || !CopyFileW(source_b, p->image_b, TRUE)) {
        error = GetLastError(); return probe_fail(p, "real_artifact_copy", error);
    }
    return probe_execute(p, p->image_a, 'A') && probe_execute(p, p->image_b, 'B');
}

int probe_open_held(ImageLeaseProbe *p) {
    DWORD error;
    if (!probe_check(p)) return 0;
    p->held = CreateFileW(p->image_a, GENERIC_READ,
                         FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                         NULL, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, NULL);
    if (p->held == INVALID_HANDLE_VALUE) {
        error = GetLastError(); return probe_fail(p, "original_read_handle", error);
    }
    return 1;
}

int probe_acquire_lease(ImageLeaseProbe *p) {
    FILE_ID_INFO original, reopened;
    DWORD error;
    p->lease = ReOpenFile(p->held, GENERIC_READ, FILE_SHARE_READ, FILE_FLAG_OPEN_REPARSE_POINT);
    if (p->lease == INVALID_HANDLE_VALUE) {
        p->lease_error = GetLastError(); return 0;
    }
    if (!GetFileInformationByHandleEx(p->held, FileIdInfo, &original, sizeof(original)) ||
        !GetFileInformationByHandleEx(p->lease, FileIdInfo, &reopened, sizeof(reopened))) {
        error = GetLastError(); return probe_fail(p, "lease_actual_file_id", error);
    }
    p->same_file = original.VolumeSerialNumber == reopened.VolumeSerialNumber &&
                   memcmp(original.FileId.Identifier, reopened.FileId.Identifier, 16) == 0;
    return p->same_file || probe_fail(p, "lease_not_original_file", ERROR_INVALID_DATA);
}

void probe_cleanup(ImageLeaseProbe *p) {
    if (p->view && !UnmapViewOfFile(p->view)) {
        DWORD error = GetLastError(); if (!p->cleanup_error) p->cleanup_error = error;
    }
    close_local(p, p->image_section);
    close_local(p, p->mapping);
    close_local(p, p->writer);
    close_local(p, p->lease);
    close_local(p, p->held);
}
