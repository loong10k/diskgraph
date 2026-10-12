#define _GNU_SOURCE
#include "linux_supervisor_spawn.h"
#include <errno.h>
#include <linux/capability.h>
#include <stdio.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <unistd.h>
#include <grp.h>

/* 单次隔离进程真实降权，不创建Engine或桌面服务。Root恢复探针只在本夹具使用。 */
int main(int argc, char **argv) {
    if (argc != 2 || getuid() || geteuid() || getgid() || getegid()) return 2;
    long result;
    if (!strcmp(argv[1], "nonroot")) {
        if (setgroups(0, NULL) || setgid(23112) || setuid(23112)) return 3;
        result = dg_linux_guardian_drop_identity(23111, 23113);
        if (result != -EPERM || getuid()!=23112 || geteuid()!=23112) return 4;
        puts("{\"nonroot_refused\":true,\"passed\":true}");
        return 0;
    }
    if (!strcmp(argv[1], "invalid_uid") || !strcmp(argv[1], "invalid_gid")) {
        int before = prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
        result = dg_linux_guardian_drop_identity(!strcmp(argv[1], "invalid_uid") ? 0 : 23111,
                                                !strcmp(argv[1], "invalid_gid") ? 0 : 23113);
        if (result != -EINVAL || getuid() || geteuid() || getgid() || getegid()
            || prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0)!=before) return 5;
        puts("{\"invalid_identity_refused_before_mutation\":true,\"passed\":true}");
        return 0;
    }
    if (strcmp(argv[1], "valid")) return 2;
    gid_t groups[2] = {23110, 23114};
    if (setgroups(2, groups)) return 6;
    result = dg_linux_guardian_drop_identity(23111, 23113);
    if (result) { fprintf(stderr, "drop errno=%ld\n", -result); return 7; }
    uid_t ur, ue, us;
    gid_t gr, ge, gs;
    if (getresuid(&ur, &ue, &us) || getresgid(&gr, &ge, &gs)) return 8;
    struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
    struct __user_cap_data_struct capabilities[2] = {{0,0,0},{0,0,0}};
    if (syscall(SYS_capget, &header, capabilities)) return 9;
    for (int i=0;i<2;i++)
        if (capabilities[i].effective || capabilities[i].permitted || capabilities[i].inheritable) return 10;
    errno=0;
    int regain=setresuid(0,0,0), denied=errno;
    int passed=ur==23111 && ue==23111 && us==23111 && gr==23113 && ge==23113 && gs==23113
        && getgroups(0,NULL)==0 && prctl(PR_GET_NO_NEW_PRIVS,0,0,0,0)==1
        && prctl(PR_GET_DUMPABLE,0,0,0,0)==0 && regain==-1 && denied==EPERM;
    printf("{\"uid\":%u,\"gid\":%u,\"groups\":%d,\"zero_capabilities\":true,"
           "\"no_new_privs\":%d,\"dumpable\":%d,\"root_regain_errno\":%d,\"passed\":%s}\n",
           ue,ge,getgroups(0,NULL),prctl(PR_GET_NO_NEW_PRIVS,0,0,0,0),
           prctl(PR_GET_DUMPABLE,0,0,0,0),denied,passed?"true":"false");
    return passed?0:11;
}
