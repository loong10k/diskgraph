#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

/* 原生 ELF 被测程序；不实现 launcher/filter/pidfd 算法。root 显式编译两份 IMAGE_ID。 */
#ifndef IMAGE_ID
#define IMAGE_ID 1
#endif

static void *alive_thread(void *unused) {
    (void)unused;
    char byte;
    for (;;) {
        puts("THREAD_HEARTBEAT");
        fflush(stdout);
        ssize_t count = read(STDIN_FILENO, &byte, 1);
        if (count == 0) return NULL;
        if (count < 0 && errno == EINTR) continue;
        if (count < 0) _exit(72);
    }
}

static int report_image(int argc, char **argv) {
    const char *fixed = getenv("DG_FIXED_ENV");
    int canary = argc > 2 ? atoi(argv[2]) : -1;
    errno = 0;
    int result = canary >= 0 ? fcntl(canary, F_GETFD) : -1;
    int saved = errno;
    printf("IMAGE=%d\nFIXED=%s\nPATH_ABSENT=%d\nLOADER_ABSENT=%d\n",
           IMAGE_ID, fixed ? fixed : "absent", getenv("PATH") == NULL,
           getenv("LD_PRELOAD") == NULL && getenv("LD_LIBRARY_PATH") == NULL);
    printf("CANARY_CLOSED=%d\n", canary < 0 || (result == -1 && saved == EBADF));
    fprintf(stderr, "STDERR_IMAGE=%d\n", IMAGE_ID);
    return 0;
}

static int echo_input(void) {
    unsigned char bytes[256];
    size_t used = 0;
    for (;;) {
        ssize_t count = read(STDIN_FILENO, bytes + used, sizeof(bytes) - used);
        if (count == 0) break;
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0 || (size_t)count > sizeof(bytes) - used) return 73;
        used += (size_t)count;
        if (used == sizeof(bytes)) return 74;
    }
    printf("COUNT=%zu\nHEX=", used);
    for (size_t index = 0; index < used; ++index) printf("%02x", bytes[index]);
    puts("\nSTDIN_EOF=true");
    return 0;
}

static int derive_attempts(void) {
    errno = 0;
    long forked = syscall(SYS_clone, (unsigned long)SIGCHLD, 0, 0, 0, 0);
    int fork_errno = errno;
    if (forked == 0) _exit(75);
    /* 约束失效仍回收本案实际创建的后代；报告原值，不能用清理把错误变成成功。 */
    printf("PROCESS_CLONE=%ld\nPROCESS_CLONE_ERRNO=%d\n", forked, fork_errno);
    if (forked > 0) {
        int status;
        while (waitpid((pid_t)forked, &status, 0) < 0) {
            if (errno != EINTR) return 81;
        }
    }
    errno = 0;
    int changed = setsid();
    printf("SETSID=%d\nSETSID_ERRNO=%d\n", changed, errno);
    errno = 0;
    long changed_group = syscall(SYS_setpgid, 0, 0);
    printf("SETPGID=%ld\nSETPGID_ERRNO=%d\n", changed_group, errno);
    pthread_t thread;
    int created = pthread_create(&thread, NULL, alive_thread, NULL);
    printf("PTHREAD_CREATE=%d\n", created);
    fflush(stdout);
    if (created != 0) return 76;
    if (pthread_join(thread, NULL) != 0) return 77;
    return 0;
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc < 2) return 70;
    if (strcmp(argv[1], "image") == 0) return report_image(argc, argv);
    if (strcmp(argv[1], "echo") == 0) return echo_input();
    if (strcmp(argv[1], "signal") == 0) {
        puts("BEFORE_DEFAULT_SIGNAL");
        raise(SIGUSR1);
        puts("INHERITED_HANDLER_WAS_USED");
        return 78;
    }
    if (strcmp(argv[1], "derive") == 0) return derive_attempts();
    if (strcmp(argv[1], "thread_exit") == 0) {
        pthread_t first, second;
        if (pthread_create(&first, NULL, alive_thread, NULL) != 0) return 79;
        if (pthread_create(&second, NULL, alive_thread, NULL) != 0) return 80;
        puts("MAIN_PTHREAD_EXIT");
        pthread_exit(NULL);
    }
    return 71;
}
