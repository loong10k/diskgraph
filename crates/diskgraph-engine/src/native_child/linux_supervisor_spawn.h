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
    uint32_t group_count;
    const uint32_t *groups; /* 受信父侧出生前固定的辅助组；零组保持guardian旧契约。 */
    int worker_fd; /* 0保持旧七FD合同；>=10的原只读worker在exec后仅交付固定FD7。 */
    uint32_t executor_protection; /* 0为guardian兼容；1检查采用客户凭据后的镜像读取拒绝。 */
};
long dg_linux_supervisor_spawn(const struct dg_supervisor_request *request,
    int *pidfd, uint64_t *old_mask, int *mask_active);
#endif
