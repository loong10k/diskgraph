#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <linux/capability.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

/* 隔离 ELF 出生边界探针，不属于产品 launcher，不创建 Engine 或容量槽。 */
struct observation { int dumpable, uid, gid, groups, capabilities_empty; };
static void lower(void) {
    struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
    struct __user_cap_data_struct data[2] = {{0}};
    if (setgroups(0, NULL) || setresgid(23111,23111,23111) ||
        setresuid(23111,23111,23111) || syscall(SYS_capset, &header, data) ||
        prctl(PR_SET_NO_NEW_PRIVS,1,0,0,0)) _exit(90);
}
static int role(void) {
    struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
    struct __user_cap_data_struct data[2] = {{0}};
    if (syscall(SYS_capget, &header, data)) return 91;
    struct observation value = {prctl(PR_GET_DUMPABLE,0,0,0,0), (int)geteuid(),
        (int)getegid(), getgroups(0,NULL),
        !(data[0].effective|data[0].permitted|data[0].inheritable|
          data[1].effective|data[1].permitted|data[1].inheritable)};
    if (write(3,&value,sizeof(value)) != (ssize_t)sizeof(value)) return 92;
    struct pollfd control = {4,POLLIN,0};
    if (poll(&control,1,5000) != 1) return 93;
    char command;
    return read(4,&command,1) == 1 && command == 'X' ? 0 : 94;
}
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1],"--role")) return role();
    if (argc != 3 || geteuid() != 0) return 80;
    if (signal(SIGPIPE,SIG_IGN)==SIG_ERR) return 79;
    int expected = atoi(argv[2]);
    if (expected != 0 && expected != 1) return 81;
    int image = open(argv[1],O_RDONLY|O_CLOEXEC|O_NOFOLLOW);
    int ready[2],control[2];
    if (image < 0 || pipe2(ready,O_CLOEXEC) || pipe2(control,O_CLOEXEC)) return 82;
    pid_t child = fork();
    if (child < 0) return 83;
    if (!child) {
        /* 固定高 FD 避免 dup2 源别名；原 parent 始终保留 wait 责任。 */
        int executable=fcntl(image,F_DUPFD_CLOEXEC,20);
        int notify=fcntl(ready[1],F_DUPFD_CLOEXEC,20);
        int receive=fcntl(control[0],F_DUPFD_CLOEXEC,20);
        if (executable<0 || notify<0 || receive<0 || dup2(notify,3)<0 || dup2(receive,4)<0) _exit(84);
        for (int fd=5;fd<20;fd++) close(fd);
        close(notify);close(receive);
        lower();
        /* 此前置标志会被普通 exec 重置，必须观察真正 exec 后的值。 */
        if (prctl(PR_SET_DUMPABLE,0,0,0,0)) _exit(85);
        char *arguments[]={argv[1],"--role",NULL};
        char *environment[]={"DG_PROBE_SENTINEL=synthetic",NULL};
        fexecve(executable,arguments,environment);
        _exit(86);
    }
    close(ready[1]);close(control[0]);close(image);
    struct observation value={-1,-1,-1,-1,-1};
    struct pollfd event={ready[0],POLLIN,0};
    int received=poll(&event,1,5000)==1 && read(ready[0],&value,sizeof(value))==(ssize_t)sizeof(value);
    pid_t observer=-1;
    if (received) observer=fork();
    if (observer==0) {
        close(ready[0]);close(control[1]);lower();
        char path[128],buffer[128]={0};
        snprintf(path,sizeof(path),"/proc/%d/environ",(int)child);
        int fd=open(path,O_RDONLY|O_CLOEXEC); int error=fd<0?errno:0;
        ssize_t count=fd<0?-1:read(fd,buffer,sizeof(buffer)-1);
        if (fd>=0) close(fd);
        int exposed=count>0 && strstr(buffer,"DG_PROBE_SENTINEL=synthetic")!=NULL;
        printf("{\"same_uid_environ_exposed\":%s,\"open_errno\":%d}\n",exposed?"true":"false",error);
        fflush(stdout);
        _exit(expected ? (exposed?0:95) : (fd<0 && error==EACCES?0:96));
    }
    int observer_status=0,child_status=0;
    int observer_waited=observer>0 && waitpid(observer,&observer_status,0)==observer;
    /* 正反例都明确推进原控制端并消费原 child wait，不用测试 kill 掩盖失败。 */
    char stop='X';if (received) (void)write(control[1],&stop,1);
    close(control[1]);close(ready[0]);
    int child_waited=waitpid(child,&child_status,0)==child;
    printf("{\"dumpable_after_exec\":%d,\"uid\":%d,\"gid\":%d,\"groups\":%d,\"capabilities_empty\":%s,\"original_child_waited\":%s,\"observer_waited\":%s}\n",
        value.dumpable,value.uid,value.gid,value.groups,value.capabilities_empty?"true":"false",child_waited?"true":"false",observer_waited?"true":"false");
    return received && value.dumpable==expected && value.uid==23111 && value.gid==23111 &&
        value.groups==0 && value.capabilities_empty && observer_waited && WIFEXITED(observer_status) &&
        WEXITSTATUS(observer_status)==0 && child_waited && WIFEXITED(child_status) && WEXITSTATUS(child_status)==0 ? 0 : 97;
}
