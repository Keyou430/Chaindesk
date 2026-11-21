cmake_minimum_required(VERSION 3.22)
project(chaindesk_jni LANGUAGES C)

set(CORE_DIRECTORY "${CMAKE_CURRENT_LIST_DIR}/../../../build/generated/jniLibs/${ANDROID_ABI}")
add_library(chaindesk_core SHARED IMPORTED)
set_target_properties(chaindesk_core PROPERTIES IMPORTED_LOCATION "${CORE_DIRECTORY}/libchaindesk_core.so")

add_library(chaindesk_jni SHARED core_jni.c)
target_include_directories(chaindesk_jni PRIVATE "${CORE_DIRECTORY}")
target_link_libraries(chaindesk_jni PRIVATE chaindesk_core)
target_link_options(chaindesk_jni PRIVATE "-Wl,-z,max-page-size=16384")
