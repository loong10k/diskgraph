/* 原生 Rust PF-06 的 Apple SDK ABI 适配层；只认证原 FD 当前卷和 ACL，不证明 fresh 历史。 */
#include <sys/acl.h>
#include <sys/attr.h>
#include <sys/fcntl.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <errno.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>

static int saved_error(void) {
    return errno != 0 ? errno : EIO;
}

static uint32_t read_u32(const unsigned char *bytes) {
    uint32_t value;
    memcpy(&value, bytes, sizeof(value));
    return value;
}

/* 显式查询 ACL present 状态，不把 NULL 或读取失败解释为无 ACL。 */
static int check_acl(int fd) {
    filesec_t security = filesec_init();
    acl_t acl = NULL;
    int result = 0;
    int present = 0;
    struct stat metadata;
    if (security == NULL) {
        return saved_error();
    }
    if (fstatx_np(fd, &metadata, security) != 0) {
        result = saved_error();
        goto finish;
    }
    if (filesec_query_property(security, FILESEC_ACL, &present) != 0) {
        result = saved_error();
        goto finish;
    }
    if (present == 0) {
        goto finish;
    }
    /* filesec 返回属性存在位掩码（ACL 为 32），不是归一化的布尔 1。 */
    if (present < 0 || filesec_get_property(security, FILESEC_ACL, &acl) != 0 || acl == NULL) {
        result = saved_error();
        goto finish;
    }
    if (acl_valid(acl) != 0) {
        result = saved_error();
        goto finish;
    }

    const acl_permset_mask_t known = ACL_READ_DATA | ACL_WRITE_DATA | ACL_EXECUTE |
        ACL_DELETE | ACL_APPEND_DATA | ACL_DELETE_CHILD | ACL_READ_ATTRIBUTES |
        ACL_WRITE_ATTRIBUTES | ACL_READ_EXTATTRIBUTES | ACL_WRITE_EXTATTRIBUTES |
        ACL_READ_SECURITY | ACL_WRITE_SECURITY | ACL_CHANGE_OWNER | ACL_SYNCHRONIZE;
    const acl_permset_mask_t mutation = ACL_WRITE_DATA | ACL_DELETE | ACL_APPEND_DATA |
        ACL_DELETE_CHILD | ACL_WRITE_ATTRIBUTES | ACL_WRITE_EXTATTRIBUTES |
        ACL_WRITE_SECURITY | ACL_CHANGE_OWNER;
    /* Darwin 成功返回 0；连续索引到尾部返回 -1/EINVAL，与 Linux ACL API 不同。 */
    for (int index = 0; index <= 128; ++index) {
        acl_entry_t entry = NULL;
        errno = 0;
        if (acl_get_entry(acl, index, &entry) != 0) {
            result = errno == EINVAL ? 0 : saved_error();
            goto finish;
        }
        if (index == 128 || entry == NULL) {
            result = ENOTSUP;
            goto finish;
        }
        acl_tag_t tag;
        acl_permset_mask_t permissions;
        if (acl_get_tag_type(entry, &tag) != 0 ||
            acl_get_permset_mask_np(entry, &permissions) != 0) {
            result = saved_error();
            goto finish;
        }
        if ((tag != ACL_EXTENDED_ALLOW && tag != ACL_EXTENDED_DENY) ||
            (permissions & ~known) != 0 ||
            (tag == ACL_EXTENDED_ALLOW && (permissions & mutation) != 0)) {
            /* 不解析 UUID/group membership；保守拒绝任何主体的变更型 ALLOW。 */
            result = ENOTSUP;
            goto finish;
        }
    }
    result = ENOTSUP;

finish:
    if (acl != NULL && acl_free(acl) != 0 && result == 0) {
        result = saved_error();
    }
    filesec_free(security);
    return result;
}

/* 参数：原 FD 与至少 24 字节输出；返回：0 成功或 errno 数字。
 * 输出为原生端序 fsid 两个 i32 加 UUID 16 字节，不向 Rust 暴露 C stat 布局。 */
int diskgraph_macos_installation_metadata(int fd, unsigned char output[24]) {
    struct statfs volume;
    if (output == NULL) {
        return EINVAL;
    }
    memset(output, 0, 24);
    if (fstatfs(fd, &volume) != 0) {
        return saved_error();
    }
    if ((volume.f_flags & MNT_LOCAL) == 0 ||
        (volume.f_flags & MNT_IGNORE_OWNERSHIP) != 0) {
        return ENOTSUP;
    }
    struct attrlist attributes = {0};
    attributes.bitmapcount = ATTR_BIT_MAP_COUNT;
    attributes.commonattr = ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_FSID;
    attributes.volattr = ATTR_VOL_INFO | ATTR_VOL_CAPABILITIES | ATTR_VOL_UUID;
    unsigned char bytes[80] = {0};
    if (fgetattrlist(fd, &attributes, bytes, sizeof(bytes), 0) != 0) {
        return saved_error();
    }
    /* 固定打包按 4 字节对齐；INFO 无返回字段，不要求 returned INFO 位。 */
    const uint32_t required_common = ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_FSID;
    const uint32_t required_volume = ATTR_VOL_CAPABILITIES | ATTR_VOL_UUID;
    if (read_u32(bytes) != sizeof(bytes) ||
        read_u32(bytes + 4) != required_common ||
        read_u32(bytes + 8) != required_volume ||
        read_u32(bytes + 12) != 0 || read_u32(bytes + 16) != 0 || read_u32(bytes + 20) != 0) {
        return ENOTSUP;
    }
    const uint32_t capability = read_u32(bytes + 36);
    const uint32_t validity = read_u32(bytes + 52);
    if ((capability & VOL_CAP_INT_EXTENDED_SECURITY) == 0 ||
        (validity & VOL_CAP_INT_EXTENDED_SECURITY) == 0 ||
        (read_u32(bytes + 32) & VOL_CAP_FMT_NO_PERMISSIONS) != 0 ||
        (read_u32(bytes + 48) & VOL_CAP_FMT_NO_PERMISSIONS) == 0 ||
        memcmp(bytes + 24, &volume.f_fsid, 8) != 0) {
        return ENOTSUP;
    }
    const unsigned char zero_uuid[16] = {0};
    if (memcmp(bytes + 64, zero_uuid, 16) == 0) {
        return ENOTSUP;
    }
    int result = check_acl(fd);
    if (result != 0) {
        return result;
    }
    memcpy(output, bytes + 24, 8);
    memcpy(output + 8, bytes + 64, 16);
    return 0;
}
