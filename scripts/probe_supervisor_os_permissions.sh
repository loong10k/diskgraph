#!/bin/sh
# 仅用于一次性 Linux root 容器，验证监督身份分离的 OS 权限事实。
# 此探针不运行 Engine、不安装服务、不证明产品身份切换已经实现。
set -eu
[ "${DISKGRAPH_ISOLATED_PERMISSION_FIXTURE:-}" = 1 ] || {
    printf '%s\n' 'explicit isolated fixture flag required' >&2
    exit 64
}
[ "$(id -u)" = 0 ] || {
    printf '%s\n' 'root-provisioned isolated fixture required' >&2
    exit 64
}
task_root=$(mktemp -d /tmp/diskgraph_os_permissions.XXXXXX)
trap 'rm -rf -- "$task_root"' EXIT HUP INT TERM
# 外层可遍历，实际权限仅由下面各自独占的根和文件决定。
chmod 755 "$task_root"
mkdir "$task_root/frontend_private" "$task_root/service_private" "$task_root/group_private"
for name in frontend_private service_private group_private; do
    printf synthetic > "$task_root/$name/file"
done
chown -R 1001:1001 "$task_root/frontend_private"
chown -R 1000:1000 "$task_root/service_private"
chmod 700 "$task_root/frontend_private" "$task_root/service_private"
chmod 600 "$task_root/frontend_private/file" "$task_root/service_private/file"
# 第三方组文件不能依靠前端 UID 或主组读取，必须保留辅助组。
chown -R 1002:2001 "$task_root/group_private"
chmod 750 "$task_root/group_private"
chmod 640 "$task_root/group_private/file"
helper_source=$(dirname "$0")/fixtures/supervisor_os_permission_probe.rs
rustc --edition 2024 "$helper_source" -o "$task_root/helper"
chmod 755 "$task_root/helper"
setpriv --reuid 1001 --regid 1001 --clear-groups "$task_root/helper" 1001 1001 none "$task_root/frontend_private/file" allow
printf '%s\n' 'frontend_reads_own_private=PASS'
setpriv --reuid 1000 --regid 1000 --clear-groups "$task_root/helper" 1000 1000 none "$task_root/frontend_private/file" deny
printf '%s\n' 'service_cannot_read_frontend_private=PASS'
setpriv --reuid 1000 --regid 1000 --clear-groups "$task_root/helper" 1000 1000 none "$task_root/service_private/file" allow
printf '%s\n' 'service_reads_own_private=PASS'
setpriv --reuid 1001 --regid 1001 --clear-groups "$task_root/helper" 1001 1001 none "$task_root/service_private/file" deny
printf '%s\n' 'frontend_cannot_read_service_private=PASS'
setpriv --reuid 1001 --regid 1001 --clear-groups "$task_root/helper" 1001 1001 none "$task_root/group_private/file" deny
printf '%s\n' 'uid_gid_without_supplementary_group_refused=PASS'
setpriv --reuid 1001 --regid 1001 --groups 2001 "$task_root/helper" 1001 1001 2001 "$task_root/group_private/file" allow
printf '%s\n' 'original_supplementary_group_reads_group_private=PASS'
