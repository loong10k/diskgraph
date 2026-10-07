#!/bin/sh
set -eu
mkdir -p /fixture/supervisor_namespace
chmod 755 /fixture /fixture/supervisor_namespace
for case in good parallel writable user_owned linked_slot symlink_slot wrong_owner writable_slot missing_slot tamper role_change; do
 mkdir /fixture/supervisor_namespace/$case
 chmod 755 /fixture/supervisor_namespace/$case
done
for case in good parallel linked_slot wrong_owner writable_slot tamper; do
 touch /fixture/supervisor_namespace/$case/uid_1000.slot
 chmod 600 /fixture/supervisor_namespace/$case/uid_1000.slot
done
for case in good parallel linked_slot writable_slot tamper; do
 chown 1000:1000 /fixture/supervisor_namespace/$case/uid_1000.slot
done
chmod 777 /fixture/supervisor_namespace/writable
chown 1000:1000 /fixture/supervisor_namespace/user_owned
ln -s good /fixture/supervisor_namespace/symlink
ln /fixture/supervisor_namespace/linked_slot/uid_1000.slot /fixture/supervisor_namespace/linked_slot/alias
ln -s ../good/uid_1000.slot /fixture/supervisor_namespace/symlink_slot/uid_1000.slot
chmod 666 /fixture/supervisor_namespace/writable_slot/uid_1000.slot
touch /fixture/supervisor_namespace/role_change/uid_0.slot
chmod 600 /fixture/supervisor_namespace/role_change/uid_0.slot
"$DG_TEST_BINARY" recovery_slot::linux_supervisor_namespace_tests::service_identity_cannot_follow_an_effective_uid_change --exact --include-ignored --test-threads=1 --nocapture > /output/role_change_green.log 2>&1
setpriv --reuid=1000 --regid=1000 --clear-groups "$DG_TEST_BINARY" recovery_slot::linux_supervisor_namespace_tests --skip service_identity_cannot_follow_an_effective_uid_change --include-ignored --test-threads=1 --nocapture > /output/namespace_green.log 2>&1
setpriv --reuid=1000 --regid=1000 --clear-groups "$DG_TEST_BINARY" recovery_slot::linux_supervisor_materials_tests --test-threads=1 --nocapture > /output/materials_green.log 2>&1
cp /fixture/supervisor_namespace/good/uid_1000.slot /output/slot_before_frontend
set +e
setpriv --reuid=1001 --regid=1001 --clear-groups dd if=/dev/zero of=/fixture/supervisor_namespace/good/uid_1000.slot bs=8 count=1 > /output/frontend_write.stdout 2> /output/frontend_write.stderr
result=$?
set -e
printf '%s\n' "$result" > /output/frontend_write.exit
test "$result" -ne 0
cmp /fixture/supervisor_namespace/good/uid_1000.slot /output/slot_before_frontend
printf '%s\n' 'actual frontend UID1001 cannot rewrite service UID1000 ACTIVE slot; original bytes retained' > /output/frontend_isolation.log
