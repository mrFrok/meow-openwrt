#include <stdio.h>
#include <qemu-plugin.h>
QEMU_PLUGIN_EXPORT int qemu_plugin_version = QEMU_PLUGIN_VERSION;
static qemu_plugin_u64 cnt;
static struct qemu_plugin_scoreboard *sb;
static void tb_trans(struct qemu_plugin_tb *tb, void *ud) {
    qemu_plugin_register_vcpu_tb_exec_inline_per_vcpu(tb, QEMU_PLUGIN_INLINE_ADD_U64, cnt, qemu_plugin_tb_n_insns(tb));
}
static void at_exit(void *p) {
    fprintf(stderr, "INSNS %llu\n", (unsigned long long)qemu_plugin_u64_sum(cnt));
}
QEMU_PLUGIN_EXPORT int qemu_plugin_install(qemu_plugin_id_t id, const qemu_info_t *info, int argc, char **argv) {
    sb = qemu_plugin_scoreboard_new(sizeof(uint64_t));
    cnt = qemu_plugin_scoreboard_u64(sb);
    qemu_plugin_register_vcpu_tb_trans_cb(id, tb_trans, NULL);
    qemu_plugin_register_atexit_cb(id, at_exit, NULL);
    return 0;
}
