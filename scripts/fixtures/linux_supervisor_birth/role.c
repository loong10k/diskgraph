#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/capability.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <poll.h>
#include <unistd.h>
#include <time.h>
#include <signal.h>

/* 固定ELF夹具，不创建Engine或授予任意命令；观察真正exec后的身份与白名单。 */
static int notification(unsigned version, unsigned sequence, const char *value) {
    char packet[516];
    int length = snprintf(packet + 4, sizeof(packet) - 4,
        "{\"version\":%u,\"session\":[7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7],\"sequence\":%u,\"notification\":%s}", version, sequence, value);
    if (length <= 0 || length >= (int)sizeof(packet) - 4) return 0;
    uint32_t prefix = htonl((uint32_t)length);
    memcpy(packet, &prefix, sizeof(prefix));
    return send(6, packet, (size_t)length + 4, MSG_NOSIGNAL | MSG_DONTWAIT) == length + 4;
}

/* 原前端时钟的剩余整毫秒；向下取整可提前拒绝，不续期。 */
static int frontend_remaining(struct timespec started) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return 0;
    int64_t elapsed = (int64_t)(now.tv_sec - started.tv_sec) * 1000000000 + now.tv_nsec - started.tv_nsec;
    int64_t remaining = 30000000000LL - elapsed;
    return remaining > 0 ? (int)(remaining / 1000000) : 0;
}

int main(int argc, char **argv) {
    if (argc == 4 && strcmp(argv[1], "frontend") == 0) {
        /* 真实公开连接测试角色：连接者与逐消息发送者可故意不同；不创建Engine或监督owner。 */
        struct timespec frontend_started = {0, 0};
        if (strcmp(argv[3], "doctor") == 0 && clock_gettime(CLOCK_MONOTONIC, &frontend_started) != 0) return 8;
        int connection = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
        struct sockaddr_un address = {.sun_family = AF_UNIX};
        if (connection < 0 || strlen(argv[2]) >= sizeof(address.sun_path)) return 8;
        memcpy(address.sun_path, argv[2], strlen(argv[2]) + 1);
        if (connect(connection, (struct sockaddr *)&address, sizeof(address)) != 0) return 8;
        const char input[] = "DG_FRONT_INPUT\n";
        int proof[2] = {-1, -1};
        if (strcmp(argv[3], "rights") == 0) {
            if (pipe2(proof, O_CLOEXEC) != 0) return 8;
            char auxiliary[CMSG_SPACE(sizeof(int))]; memset(auxiliary, 0, sizeof(auxiliary));
            struct iovec vector = {.iov_base = (void *)input, .iov_len = sizeof(input) - 1};
            struct msghdr message = {.msg_iov = &vector, .msg_iovlen = 1, .msg_control = auxiliary, .msg_controllen = sizeof(auxiliary)};
            struct cmsghdr *header = CMSG_FIRSTHDR(&message);
            header->cmsg_level = SOL_SOCKET; header->cmsg_type = SCM_RIGHTS; header->cmsg_len = CMSG_LEN(sizeof(int));
            memcpy(CMSG_DATA(header), &proof[1], sizeof(int));
            if (sendmsg(connection, &message, MSG_NOSIGNAL) != (ssize_t)sizeof(input) - 1) return 8;
            close(proof[1]);
        } else if (strcmp(argv[3], "delegate") == 0) {
            pid_t writer = fork();
            if (writer < 0) return 8;
            if (writer == 0) _exit(send(connection, input, sizeof(input) - 1, MSG_NOSIGNAL) == (ssize_t)sizeof(input) - 1 ? 0 : 8);
            int status; if (waitpid(writer, &status, 0) != writer || !WIFEXITED(status) || WEXITSTATUS(status) != 0) return 8;
        } else {
            ssize_t sent = send(connection, input, sizeof(input) - 1, MSG_NOSIGNAL);
            if (sent != (ssize_t)sizeof(input) - 1 && errno != EPIPE && errno != ECONNRESET) return 8;
        }
        shutdown(connection, SHUT_WR);
        if (strcmp(argv[3], "doctor") == 0) {
            /* 独立前端只复制实际业务字节；原30秒总期限不随分片续期，真实EOF才正常退出。 */
            int output_flags = fcntl(STDOUT_FILENO, F_GETFL);
            if (output_flags < 0 || fcntl(STDOUT_FILENO, F_SETFL, output_flags | O_NONBLOCK) != 0 || signal(SIGPIPE, SIG_IGN) == SIG_ERR) return 8;
            unsigned total = 0;
            for (;;) {
                int remaining = frontend_remaining(frontend_started);
                struct pollfd input_ready = {.fd = connection, .events = POLLIN | POLLHUP};
                if (!remaining || poll(&input_ready, 1, remaining) <= 0) return 8;
                char bytes[4096];
                ssize_t count = recv(connection, bytes, sizeof(bytes), 0);
                if (count < 0 && errno == EINTR) continue;
                if (count < 0) return 8;
                if (!count) {
                    close(connection);
                    return total ? 0 : 8;
                }
                if ((unsigned)count > 131072 - total) return 8;
                total += (unsigned)count;
                size_t sent = 0;
                while (sent < (size_t)count) {
                    remaining = frontend_remaining(frontend_started);
                    if (!remaining) return 8;
                    ssize_t written = write(STDOUT_FILENO, bytes + sent, (size_t)count - sent);
                    if (written > 0) { sent += (size_t)written; continue; }
                    if (written < 0 && errno == EINTR) continue;
                    if (written < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
                        struct pollfd output_ready = {.fd = STDOUT_FILENO, .events = POLLOUT};
                        int ready = poll(&output_ready, 1, remaining);
                        if (ready < 0 && errno == EINTR) continue;
                        if (ready > 0) continue;
                    }
                    return 8;
                }
            }
        }
        struct pollfd waiting = {.fd = connection, .events = POLLIN | POLLHUP};
        if (poll(&waiting, 1, 5000) <= 0) return 8;
        char output[64]; ssize_t received = recv(connection, output, sizeof(output), 0);
        if (strcmp(argv[3], "plain") == 0 && (received != 15 || memcmp(output, "DG_FRONT_OUTPUT", 15) != 0)) return 8;
        if (strcmp(argv[3], "plain") != 0 && (received > 0 || (received < 0 && errno != ECONNRESET))) return 8;
        if (proof[0] >= 0) {
            waiting.fd = proof[0]; waiting.revents = 0;
            if (poll(&waiting, 1, 5000) <= 0 || read(proof[0], output, 1) != 0) return 8;
            printf("{\"unexpected_fd_closed\":true}\n"); close(proof[0]);
        }
        close(connection); return 0;
    }
    struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
    struct __user_cap_data_struct data[2] = {{0}, {0}};
    int cap_ok = syscall(SYS_capget, &header, data) == 0;
    for (int n = 0; n < 2; ++n)
        cap_ok = cap_ok && !data[n].effective && !data[n].permitted && !data[n].inheritable;
    uid_t real_uid, effective_uid, saved_uid;
    gid_t real_gid, effective_gid, saved_gid;
    int saved_ids = getresuid(&real_uid, &effective_uid, &saved_uid) == 0 && getresgid(&real_gid, &effective_gid, &saved_gid) == 0 && saved_uid == 23111 && saved_gid == 23111;
    gid_t actual_groups[2] = {0, 0};
    int supplementary = argc == 2 && strcmp(argv[1], "supplementary") == 0;
    int groups_match = supplementary ? getgroups(2, actual_groups) == 2 &&
        actual_groups[0] == 23112 && actual_groups[1] == 23113 : getgroups(0, NULL) == 0;
    int identities = saved_ids && getuid() == 23111 && geteuid() == 23111 &&
        getgid() == 23111 && getegid() == 23111 && groups_match;
    int endpoints = fcntl(5, F_GETFD) >= 0 && fcntl(6, F_GETFD) >= 0;
    int image_closed = fcntl(3, F_GETFD) == -1 && errno == EBADF;
    int startup_closed = fcntl(4, F_GETFD) == -1 && errno == EBADF;
    int whitelist = image_closed && startup_closed;
    int held_worker = argc == 3 && strcmp(argv[1], "held_worker") == 0;
    int worker_ok = 1;
    int path_denied = 0;
    if (held_worker) {
        // 路径必须拒绝，原已打开只读FD仍可读；只检查隔离synthetic镜像，不读取业务文件。
        int reopened = open(argv[2], O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
        path_denied = reopened < 0 && errno == EACCES;
        if (reopened >= 0) close(reopened);
        char bytes[15] = {0};
        int flags = fcntl(7, F_GETFD);
        worker_ok = path_denied && flags >= 0 && pread(7, bytes, sizeof(bytes), 0) == 15 &&
            memcmp(bytes, "DG_HELD_WORKER\n", 15) == 0 && pread(7, bytes, 1, 15) == 0 &&
            fcntl(7, F_SETFD, flags | FD_CLOEXEC) == 0;
    }
    for (int fd = held_worker ? 8 : 7; fd < 128; ++fd) {
        int status = fcntl(fd, F_GETFD);
        whitelist = whitelist && status == -1 && errno == EBADF;
    }
    int no_new_privileges = prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) == 1;
    int passed = identities && endpoints && cap_ok && no_new_privileges && whitelist && worker_ok;
    if (held_worker)
        printf("{\"passed\":%s,\"held_worker_read\":%s,\"worker_path_eacces\":%s}\n",
            passed ? "true" : "false", worker_ok ? "true" : "false", path_denied ? "true" : "false");
    else printf("{\"passed\":%s,\"uid\":%u,\"gid\":%u,\"groups\":%d,\"private_endpoints\":%s,\"capabilities_empty\":%s,\"whitelist\":%s,\"no_new_privs\":%s}\n",
        passed ? "true" : "false", getuid(), getgid(), getgroups(0, NULL),
        endpoints ? "true" : "false", cap_ok ? "true" : "false",
        whitelist ? "true" : "false", no_new_privileges ? "true" : "false");
    if (argc == 2 && (strcmp(argv[1], "notify") == 0 || strcmp(argv[1], "notify2") == 0 || strcmp(argv[1], "notify_input") == 0)) {
        unsigned version = strcmp(argv[1], "notify") == 0 ? 1 : 2;
        if (strcmp(argv[1], "notify_input") == 0) {
            /* 固定输入必须逐字节一致且实际EOF；公共stdin结束不证明私有退休或原wait。 */
            char input[64];
            size_t used = 0;
            for (;;) {
                ssize_t count = read(0, input + used, sizeof(input) - used);
                if (count < 0 && errno == EINTR) continue;
                if (count < 0) return 6;
                if (count == 0) break;
                used += (size_t)count;
                if (used == sizeof(input)) return 6;
            }
            const char expected[] = "DG_FOREGROUND_INPUT\n";
            if (used != sizeof(expected) - 1 || memcmp(input, expected, used) != 0) return 6;
            printf("{\"stdin_exact\":true,\"stdin_eof\":true}\n");
        }
        if (!passed || fflush(stdout) != 0 ||
            !notification(version, 0, "{\"kind\":\"ready\"}") ||
            !notification(version, 1, "{\"kind\":\"foreground_ended\",\"exit_code\":0,\"panicked\":false}") ||
            !notification(version, 2, "{\"kind\":\"cleanup_complete\"}") || close(6) != 0) return 4;
        /* 已给出私有完整流，仍以真实datagram屏障保留原进程，验证通知不能代替wait。 */
        /* 生产材料端继承O_NONBLOCK；仅此固定角色夹具显式采用阻塞屏障，不能把EAGAIN视为材料已到。 */
        int flags = fcntl(5, F_GETFL);
        if (flags < 0 || fcntl(5, F_SETFL, flags & ~O_NONBLOCK) != 0) return 4;
        char material[40];
        ssize_t received;
        do { received = recv(5, material, sizeof(material), MSG_TRUNC); } while (received < 0 && errno == EINTR);
        if (received != 40 || memcmp(material, "DGSM01A\n", 8) != 0) return 5;
        for (size_t index = 8; index < sizeof(material); ++index) if ((unsigned char)material[index] != 7) return 5;
    }
    return passed ? 0 : 3;
}
