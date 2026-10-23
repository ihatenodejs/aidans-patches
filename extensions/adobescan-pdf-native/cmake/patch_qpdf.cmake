file(READ "${SRC_DIR}/CMakeLists.txt" CONTENT)
string(REPLACE "add_subdirectory(compare-for-test)" "# add_subdirectory(compare-for-test)" CONTENT "${CONTENT}")
string(REPLACE "add_subdirectory(qpdf)" "# add_subdirectory(qpdf)" CONTENT "${CONTENT}")
string(REPLACE "add_subdirectory(libtests)" "# add_subdirectory(libtests)" CONTENT "${CONTENT}")
string(REPLACE "add_subdirectory(examples)" "# add_subdirectory(examples)" CONTENT "${CONTENT}")
string(REPLACE "add_subdirectory(zlib-flate)" "# add_subdirectory(zlib-flate)" CONTENT "${CONTENT}")
string(REPLACE "add_subdirectory(manual)" "# add_subdirectory(manual)" CONTENT "${CONTENT}")
string(REPLACE "add_subdirectory(fuzz)" "# add_subdirectory(fuzz)" CONTENT "${CONTENT}")
file(WRITE "${SRC_DIR}/CMakeLists.txt" "${CONTENT}")

# Also patch libqpdf/CMakeLists.txt to ensure static PIC
file(READ "${SRC_DIR}/libqpdf/CMakeLists.txt" LIB_CONTENT)
string(REPLACE "set(OBJECT_LIB_IS_PIC OFF)" "set(OBJECT_LIB_IS_PIC ON)" LIB_CONTENT "${LIB_CONTENT}")
file(WRITE "${SRC_DIR}/libqpdf/CMakeLists.txt" "${LIB_CONTENT}")
