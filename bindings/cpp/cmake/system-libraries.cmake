# What the Rust library links against, from `cargo rustc -- --print native-static-libs`.
if(WIN32)
  set(SIMLINK_SOURCE_SDK_SYSTEM_LIBRARIES kernel32 ntdll userenv ws2_32 dbghelp)
elseif(APPLE)
  set(SIMLINK_SOURCE_SDK_SYSTEM_LIBRARIES iconv)
else()
  set(SIMLINK_SOURCE_SDK_SYSTEM_LIBRARIES gcc_s util rt pthread m dl)
endif()
