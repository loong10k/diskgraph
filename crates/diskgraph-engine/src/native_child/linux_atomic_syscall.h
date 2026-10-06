#ifndef DG_LINUX_ATOMIC_SYSCALL_H
#define DG_LINUX_ATOMIC_SYSCALL_H
#include <stdint.h>

/* 无 libc/TLS/errno/PLT；仅 native64 little-endian 两种已规定的 syscall ABI。 */
#if defined(__x86_64__) && !defined(__ILP32__)
static __attribute__((always_inline)) inline long dg_raw(long nr, long a, long b,
        long c, long d, long e, long f) {
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    register long r9 __asm__("r9") = f;
    long result;
    __asm__ volatile("syscall" : "=a"(result)
        : "a"(nr), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory", "cc");
    return result;
}
#elif defined(__aarch64__) && __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
static __attribute__((always_inline)) inline long dg_raw(long nr, long a, long b,
        long c, long d, long e, long f) {
    register long x8 __asm__("x8") = nr;
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    register long x3 __asm__("x3") = d;
    register long x4 __asm__("x4") = e;
    register long x5 __asm__("x5") = f;
    __asm__ volatile("svc 0" : "+r"(x0)
        : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5)
        : "memory", "cc");
    return x0;
}
#else
#error "Linux atomic launcher requires native64 x86_64 or little-endian aarch64"
#endif
#endif
