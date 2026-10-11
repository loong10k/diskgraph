#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/capability.h>
#include <stdio.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <unistd.h>

/* 固定ELF夹具，不创建Engine或授予任意命令；观察真正exec后的身份与白名单。 */
int main(void) {
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
    return passed ? 0 : 3;
}
