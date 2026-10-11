#ifndef DG_LINUX_SUPERVISOR_SPAWN_H
#define DG_LINUX_SUPERVISOR_SPAWN_H
#include <stdint.h>

/* 仅受信root出生层准备；私有源FD>=10，argv/envp在出生前完成白名单与字节预算校验。
 * 本接口不签发镜像信任、不认领bootstrap、不接受前端命令；原pidfd必须立即被唯一owner采用。 */
struct dg_supervisor_request {
    int fds[7]; /* stdin/stdout/stderr/held ELF/startup/materials/private control */
    uint32_t uid, gid;
    char *const *argv;
    char *const *envp;
};
long dg_linux_supervisor_spawn(const struct dg_supervisor_request *request,
    int *pidfd, uint64_t *old_mask, int *mask_active);
#endif
