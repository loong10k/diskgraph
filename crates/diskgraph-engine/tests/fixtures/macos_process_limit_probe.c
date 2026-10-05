/* macOS 非 root helper 的实际资源限制资格；只改变本探针，不改变宿主。 */
#include <errno.h>
#include <dlfcn.h>
#include <pthread.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
extern char **environ;

static void *thread_marker(void *value) { return value; }

static int wait_original(pid_t pid) {
    int status;
    pid_t waited;
    do { waited = waitpid(pid, &status, 0); } while (waited < 0 && errno == EINTR);
    return waited == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

static int qualification(const char *self) {
    struct rlimit limited = {0, 0}, observed = {0, 0}, raised = {1, 1};
    pid_t pid;
    pid_t (*legacy_vfork)(void);
    pthread_t thread;
    void *joined = NULL;
    int token = 1, fork_error, vfork_error, spawn_error, raise_error, thread_error, join_error;
    char *spawn_args[] = {(char *)self, "spawn-child", NULL};
    char *exec_args[] = {(char *)self, "exec-inherited", NULL};
    if (getuid() == 0 || geteuid() == 0) return 91;
    /* 已弃用但仍可被旧程序调用的公开入口，动态取符号用于反控；缺失即拒绝资格。 */
    legacy_vfork = (pid_t (*)(void))dlsym(RTLD_DEFAULT, "vfork");
    if (!legacy_vfork) return 104;
    /* 同一可执行文件正控，在限额安装前实际派生和回收。 */
    pid = fork();
    if (pid == 0) _exit(0);
    if (pid < 0 || !wait_original(pid)) return 92;
    pid = legacy_vfork();
    if (pid == 0) _exit(0);
    if (pid < 0 || !wait_original(pid)) return 105;
    spawn_error = posix_spawn(&pid, self, NULL, NULL, spawn_args, environ);
    if (spawn_error != 0 || !wait_original(pid)) return 93;
    if (setrlimit(RLIMIT_NPROC, &limited) != 0 || getrlimit(RLIMIT_NPROC, &observed) != 0 ||
        observed.rlim_cur != 0 || observed.rlim_max != 0) return 94;
    errno = 0;
    pid = fork();
    if (pid == 0) _exit(0);
    fork_error = pid < 0 ? errno : 0;
    if (pid >= 0) { (void)wait_original(pid); return 95; }
    errno = 0;
    pid = legacy_vfork();
    if (pid == 0) _exit(0);
    vfork_error = pid < 0 ? errno : 0;
    if (pid >= 0) { (void)wait_original(pid); return 96; }
    spawn_error = posix_spawn(&pid, self, NULL, NULL, spawn_args, environ);
    if (spawn_error == 0) { (void)wait_original(pid); return 97; }
    errno = 0;
    raise_error = setrlimit(RLIMIT_NPROC, &raised) == 0 ? 0 : errno;
    if (raise_error == 0) return 98;
    if (getrlimit(RLIMIT_NPROC, &observed) != 0 || observed.rlim_cur != 0 || observed.rlim_max != 0) return 99;
    /* 真实 pthread 的创建和 join，不能用进程派生失败推断扫描线程可用。 */
    thread_error = pthread_create(&thread, NULL, thread_marker, &token);
    join_error = thread_error == 0 ? pthread_join(thread, &joined) : -1;
    printf("{\"phase\":\"before_exec\",\"nonroot\":true,\"positive_fork_wait\":true,"
           "\"positive_vfork_wait\":true,\"positive_spawn_wait\":true,\"fork_errno\":%d,\"vfork_errno\":%d,"
           "\"spawn_errno\":%d,\"raise_errno\":%d,\"thread_error\":%d,"
           "\"join_error\":%d,\"thread_marker\":%s}\n",
           fork_error, vfork_error, spawn_error, raise_error, thread_error, join_error,
           joined == &token ? "true" : "false");
    if (fflush(stdout) != 0 || fork_error != EAGAIN || vfork_error != EAGAIN ||
        spawn_error != EAGAIN || raise_error != EPERM || thread_error != 0 ||
        join_error != 0 || joined != &token) return 100;
    /* 同一原进程 exec 自有镜像，再核验继承；不创建新进程或刷新资格。 */
    execv(self, exec_args);
    return 101;
}

int main(int argc, char **argv) {
    struct rlimit observed = {0, 0};
    if (argc == 2 && strcmp(argv[1], "spawn-child") == 0) return 0;
    if (argc == 2 && strcmp(argv[1], "exec-inherited") == 0) {
        if (getuid() == 0 || geteuid() == 0 || getrlimit(RLIMIT_NPROC, &observed) != 0 ||
            observed.rlim_cur != 0 || observed.rlim_max != 0) return 102;
        puts("{\"phase\":\"after_exec\",\"nonroot\":true,\"soft_zero\":true,\"hard_zero\":true}");
        return fflush(stdout) == 0 ? 0 : 103;
    }
    if (argc != 1) return 90;
    return qualification(argv[0]);
}
