/*
 * Sparc cpu parameters for qemu.
 *
 * SPDX-License-Identifier: LGPL-2.0-or-later
 */

#ifndef SPARC_CPU_PARAM_H
#define SPARC_CPU_PARAM_H

#ifdef TARGET_SPARC64
# define TARGET_PAGE_BITS 13 /* 8k */
# define TARGET_PHYS_ADDR_SPACE_BITS  41
# ifdef TARGET_ABI32
#  define TARGET_VIRT_ADDR_SPACE_BITS 32
# else
#  define TARGET_VIRT_ADDR_SPACE_BITS 44
# endif
#else
# define TARGET_PAGE_BITS 12 /* 4k */
# define TARGET_PHYS_ADDR_SPACE_BITS 36
# define TARGET_VIRT_ADDR_SPACE_BITS 32
#endif

#define TARGET_INSN_START_EXTRA_WORDS 1

#endif
