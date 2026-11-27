file(READ "${SRC_DIR}/CMakeLists.txt" CONTENT)
string(REPLACE "message(FATAL_ERROR \"The \${CMAKE_PROJECT_NAME} build system cannot be integrated into another build system using add_subdirectory().  Use ExternalProject_Add() instead.\")" "message(STATUS \"libjpeg-turbo integrated via FetchContent\")" NEW_CONTENT "${CONTENT}")
# libjpeg-turbo 3.2.1 resolves its try_run source against CMAKE_SOURCE_DIR, which
# points at the consuming project when built through FetchContent. Redirect it to
# the fetched source tree. This is a no-op if a later release fixes the path.
string(REPLACE "\${CMAKE_SOURCE_DIR}/cmakescripts/right_shift_is_unsigned.c" "\${CMAKE_CURRENT_SOURCE_DIR}/cmakescripts/right_shift_is_unsigned.c" NEW_CONTENT "${NEW_CONTENT}")
file(WRITE "${SRC_DIR}/CMakeLists.txt" "${NEW_CONTENT}")
