cmake_minimum_required(VERSION 3.20)
project(sync_iobs_selftest LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# MSVC 默认不把「无 BOM 的 UTF-8」当 UTF-8：源码里的中文会让后续 #include 解析异常
# （表现为 std::array 之类明明 include 了却报未定义）。所有中文源码必须加 /utf-8。
if(MSVC)
    add_compile_options(/utf-8)
endif()

# 只编不依赖 GUI / 网络的模块，纯命令行即可验证核心算法
add_executable(selftest
    selftest.cpp
    ${CMAKE_CURRENT_SOURCE_DIR}/../src/sha1.cpp
    ${CMAKE_CURRENT_SOURCE_DIR}/../src/base64.cpp
    ${CMAKE_CURRENT_SOURCE_DIR}/../src/token.cpp
    ${CMAKE_CURRENT_SOURCE_DIR}/../src/config.cpp
)
target_include_directories(selftest PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/../src)

enable_testing()
add_test(NAME selftest COMMAND selftest)
