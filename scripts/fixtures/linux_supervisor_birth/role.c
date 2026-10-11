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
#include <unistd.h>

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

int main(int argc, char **argv) {
    struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
    struct __user_cap_data_struct data[2] = {{0}, {0}};
    int cap_ok = syscall(SYS_capget, &header, data) == 0;
    for (int n = 0; n < 2; ++n)
        cap_ok = cap_ok && !data[n].effective && !data[n].permitted && !data[n].inheritable;
    uid_t real_uid, effective_uid, saved_uid;
    gid_t real_gid, effective_gid, saved_gid;
    int saved_ids = getresuid(&real_uid, &effective_uid, &saved_uid) == 0 && getresgid(&real_gid, &effective_gid, &saved_gid) == 0 && saved_uid == 23111 && saved_gid == 23111;
    int identities = saved_ids && getuid() == 23111 && geteuid() == 23111 &&
        getgid() == 23111 && getegid() == 23111 && getgroups(0, NULL) == 0;
    int endpoints = fcntl(5, F_GETFD) >= 0 && fcntl(6, F_GETFD) >= 0;
    int image_closed = fcntl(3, F_GETFD) == -1 && errno == EBADF;
    int startup_closed = fcntl(4, F_GETFD) == -1 && errno == EBADF;
    int whitelist = image_closed && startup_closed;
    for (int fd = 7; fd < 128; ++fd) {
        int status = fcntl(fd, F_GETFD);
        whitelist = whitelist && status == -1 && errno == EBADF;
    }
    int no_new_privileges = prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) == 1;
    int passed = identities && endpoints && cap_ok && no_new_privileges && whitelist;
    printf("{\"passed\":%s,\"uid\":%u,\"gid\":%u,\"groups\":%d,\"private_endpoints\":%s,\"capabilities_empty\":%s,\"whitelist\":%s,\"no_new_privs\":%s}\n",
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
