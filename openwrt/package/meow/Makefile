# SPDX-License-Identifier: GPL-3.0-only
#
# meow-openwrt: unofficial OpenWrt build of meow-rs (see NOTICE-FORK.md).
#
# Three mutually exclusive packages, one per cargo bundle of meow-app:
#   meow-tiny  (tiny)        stock 16 MB flash routers
#   meow       (lightweight) tiny + anytls/vmess/shadow-tls
#   meow-full  (full)        everything upstream ships
# Dev build from this checkout (symlink this dir into an SDK's package/):
#   make package/meow/compile V=s
# Size experiments: MEOW_FEATURES / MEOW_EXTRA_RUSTFLAGS / MEOW_BUILD_STD=0 ...
# (openwrt-fork/bench/size-sdk.sh). Changing them needs package/meow/clean.

include $(TOPDIR)/rules.mk

PKG_NAME:=meow
PKG_VERSION:=0.28.0
PKG_RELEASE:=1

PKG_MAINTAINER:=mrFrok
PKG_LICENSE:=GPL-3.0-only
PKG_LICENSE_FILES:=LICENSE LICENSE-MIT NOTICE-FORK.md

PKG_BUILD_DEPENDS:=rust/host
PKG_BUILD_PARALLEL:=1

# Source: the checkout this Makefile lives in (realpath follows the SDK symlink).
MEOW_SRC_DIR ?= $(realpath $(CURDIR)/../../..)

include $(INCLUDE_DIR)/package.mk
include $(TOPDIR)/feeds/packages/lang/rust/rust-package.mk

# Cargo bundle = build variant name, unless overridden for an experiment.
MEOW_FEATURES ?= $(BUILD_VARIANT)
RUST_PKG_FEATURES:=$(MEOW_FEATURES)

# Size knobs (openwrt-fork/BENCHMARKS.md). The size profiles rebuild std with
# -Cpanic=immediate-abort (a panic aborts without a message; ~1 MB on mipsel),
# via nightly-only flags on the SDK's stable rustc (RUSTC_BOOTSTRAP).
ifeq ($(BUILD_VARIANT),full)
  MEOW_BUILD_STD ?= 0
else
  MEOW_BUILD_STD ?= 1
endif
MEOW_STD_FEATURES ?= optimize_for_size
MEOW_OPT_LEVEL ?= z
MEOW_EXTRA_RUSTFLAGS ?=
# PGO: an llvm-profdata file from openwrt-fork/bench/pgo-gen.sh. Profiles are
# per source version; stale functions just lose the optimisation.
MEOW_PGO_PROFILE ?=
# Fully static binary (musl + libgcc linked in): runs on any OpenWrt release
# whatever its musl, no libgcc dependency. MIPS is non-PIE (see below).
MEOW_STATIC ?= 1
# boring-sys: link libstdc++ statically (BoringSSL needs a few symbols; the
# shared libstdcpp package would cost ~2 MB of flash).
MEOW_CPPLIB ?= static=stdc++

# OpenWrt's musl 1.2 has a 64-bit time_t on 32-bit targets; the libc crate
# still defaults to the old 32-bit one there (mismatch with bindgen'd boring-sys).
ifndef CONFIG_ARCH_64BIT
  CARGO_RUSTFLAGS += --cfg=libc_unstable_musl_v1_2
endif
# MIPS PIC code goes through the GOT for every call: non-PIE is ~14% smaller.
ifneq ($(filter mips mipsel,$(ARCH)),)
  CARGO_RUSTFLAGS += -Crelocation-model=static
endif
# Overrides rust-package.mk's -crt-static (static-pie, or plain static on
# non-PIE MIPS).
ifeq ($(MEOW_STATIC),1)
  CARGO_RUSTFLAGS += -Ctarget-feature=+crt-static
  # BoringSSL's static libstdc++ calls __sync_synchronize on MIPS (libgcc_s
  # gave it when dynamic): libgcc again after it.
  ifneq ($(filter mips mipsel,$(ARCH)),)
    CARGO_RUSTFLAGS += -Clink-arg=-lgcc
  endif
endif
CARGO_RUSTFLAGS += -Cforce-unwind-tables=no -Lnative=$(TOOLCHAIN_DIR)/lib $(MEOW_EXTRA_RUSTFLAGS)
ifneq ($(MEOW_PGO_PROFILE),)
  CARGO_RUSTFLAGS += -Cprofile-use=$(MEOW_PGO_PROFILE)
endif
ifeq ($(MEOW_BUILD_STD),1)
  CARGO_RUSTFLAGS += -Zunstable-options -Cpanic=immediate-abort -Zlocation-detail=none
  CARGO_PKG_CONFIG_VARS += RUSTC_BOOTSTRAP=1
  MEOW_CARGO_ARGS := -Zbuild-std=std,panic_abort \
	$(if $(MEOW_STD_FEATURES),-Zbuild-std-features=$(MEOW_STD_FEATURES))
endif

# Keep the workspace release profile (panic=abort); rust-values.mk forces unwind.
CARGO_PKG_CONFIG_VARS += \
	CARGO_PROFILE_RELEASE_PANIC=abort \
	CARGO_PROFILE_RELEASE_OPT_LEVEL=$(MEOW_OPT_LEVEL) \
	CARGO_PROFILE_RELEASE_STRIP=true \
	TARGET_CXX=$(TARGET_CXX_NOCACHE) \
	TARGET_AR=$(TARGET_AR) \
	BORING_BSSL_RUST_CPPLIB_$(subst -,_,$(RUSTC_TARGET_ARCH))=$(MEOW_CPPLIB) \
	TARGET_CXXFLAGS="$(TARGET_CXXFLAGS) $(RUSTC_CFLAGS)" \
	CMAKE_TOOLCHAIN_FILE_$(subst -,_,$(RUSTC_TARGET_ARCH))=$(PKG_BUILD_DIR)/openwrt-toolchain.cmake \
	BINDGEN_EXTRA_CLANG_ARGS_$(subst -,_,$(RUSTC_TARGET_ARCH))="--sysroot=$(TOOLCHAIN_DIR) -isystem $(STAGING_DIR)/usr/include"

define Package/meow/Default
  SECTION:=net
  CATEGORY:=Network
  SUBMENU:=Web Servers/Proxies
  URL:=https://github.com/mrFrok/meow-openwrt
  DEPENDS:=$(RUST_ARCH_DEPENDS)
  PROVIDES:=meow-core
endef

define Package/meow-tiny
  $(call Package/meow/Default)
  TITLE:=Clash Meta compatible proxy kernel (meow-rs), tiny profile
  VARIANT:=tiny
  CONFLICTS:=meow meow-full
endef

define Package/meow
  $(call Package/meow/Default)
  TITLE:=Clash Meta compatible proxy kernel (meow-rs)
  VARIANT:=lightweight
  DEFAULT_VARIANT:=1
  CONFLICTS:=meow-full
endef

define Package/meow-full
  $(call Package/meow/Default)
  TITLE:=Clash Meta compatible proxy kernel (meow-rs), all protocols
  VARIANT:=full
endef

define Package/meow/description/Default
  Unofficial OpenWrt build of meow-rs, a Rust implementation of the
  mihomo (Clash Meta) proxy kernel.
endef

define Package/meow-tiny/description
$(call Package/meow/description/Default)
  Tiny profile for 16 MB flash: Trojan, VLESS (Vision, Reality),
  Hysteria2, mux, DoH/DoT, mixed/tproxy/tun inbounds. Shadowsocks nodes
  load as unavailable; use the meow package for them.
endef

define Package/meow/description
$(call Package/meow/description/Default)
  Tiny profile plus Shadowsocks, AnyTLS, VMess and shadow-tls.
endef

define Package/meow-full/description
$(call Package/meow/description/Default)
  Every protocol, transport and inbound upstream ships.
endef

define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	rsync -a --delete --exclude=/target --exclude=/.git $(MEOW_SRC_DIR)/ $(PKG_BUILD_DIR)/
	rm -f $(PKG_BUILD_DIR)/rust-toolchain.toml
endef

# boring-sys has no built-in cross setup for MIPS: hand cmake the SDK toolchain.
define Build/Compile
	printf '%s\n' \
		'set(CMAKE_SYSTEM_NAME Linux)' \
		'set(CMAKE_SYSTEM_PROCESSOR $(ARCH))' \
		'set(CMAKE_C_COMPILER $(TARGET_CC_NOCACHE))' \
		'set(CMAKE_CXX_COMPILER $(TARGET_CXX_NOCACHE))' \
		'set(CMAKE_ASM_COMPILER $(TARGET_CC_NOCACHE))' \
		'set(CMAKE_FIND_ROOT_PATH $(STAGING_DIR))' \
		'set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)' \
		'set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)' \
		'set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)' \
		'set(BUILD_TESTING OFF CACHE BOOL "" FORCE)' \
		> $(PKG_BUILD_DIR)/openwrt-toolchain.cmake
	$(call Build/Compile/Cargo,crates/meow-app,--no-default-features --bin meow $(MEOW_CARGO_ARGS))
endef

define Package/meow/install/Default
	$(INSTALL_DIR) $(1)/usr/bin
	$(INSTALL_BIN) $(PKG_INSTALL_DIR)/bin/meow $(1)/usr/bin/meow
endef
Package/meow-tiny/install = $(Package/meow/install/Default)
Package/meow/install = $(Package/meow/install/Default)
Package/meow-full/install = $(Package/meow/install/Default)

$(eval $(call BuildPackage,meow-tiny))
$(eval $(call BuildPackage,meow))
$(eval $(call BuildPackage,meow-full))
