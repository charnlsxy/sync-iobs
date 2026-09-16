cmake_minimum_required(VERSION 3.20)
project(sync_iobs VERSION 0.1.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()

# Windows 静态链接 CRT，产物可单文件分发（MSVC）
if(MSVC)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

include(FetchContent)

# ---- Dear ImGui（官方无 CMake，这里手动建静态库）----
FetchContent_Declare(imgui
    GIT_REPOSITORY https://github.com/ocornut/imgui.git
    GIT_TAG        v1.92.9b
    GIT_SHALLOW    TRUE
    GIT_PROGRESS   TRUE)

# ---- GLFW（窗口 + OpenGL 上下文）----
set(GLFW_BUILD_DOCS     OFF CACHE BOOL "" FORCE)
set(GLFW_BUILD_TESTS    OFF CACHE BOOL "" FORCE)
set(GLFW_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(GLFW_INSTALL        OFF CACHE BOOL "" FORCE)
# GLFW 3.4 默认启用 Wayland，需要 wayland-scanner/protocols；
# 本项目只用 X11（Wayland 桌面走 XWayland），关掉可省一堆构建依赖
set(GLFW_BUILD_WAYLAND  OFF CACHE BOOL "" FORCE)
set(GLFW_BUILD_X11      ON  CACHE BOOL "" FORCE)
FetchContent_Declare(glfw
    GIT_REPOSITORY https://github.com/glfw/glfw.git
    GIT_TAG        3.4
    GIT_SHALLOW    TRUE
    GIT_PROGRESS   TRUE)

FetchContent_MakeAvailable(imgui glfw)

add_library(imgui STATIC
    ${imgui_SOURCE_DIR}/imgui.cpp
    ${imgui_SOURCE_DIR}/imgui_draw.cpp
    ${imgui_SOURCE_DIR}/imgui_tables.cpp
    ${imgui_SOURCE_DIR}/imgui_widgets.cpp
    ${imgui_SOURCE_DIR}/backends/imgui_impl_glfw.cpp
    ${imgui_SOURCE_DIR}/backends/imgui_impl_opengl3.cpp)
target_include_directories(imgui PUBLIC
    ${imgui_SOURCE_DIR}
    ${imgui_SOURCE_DIR}/backends)
target_link_libraries(imgui PUBLIC glfw)

find_package(OpenGL REQUIRED)
target_link_libraries(imgui PUBLIC OpenGL::GL)

if(APPLE)
    target_compile_definitions(imgui PUBLIC GL_SILENCE_DEPRECATION)
endif()

# ---- 主程序 ----
# 骨架阶段只编 main.cpp；后续移植各模块时在这里逐步追加源文件
add_executable(sync-iobs
    src/main.cpp
)
target_include_directories(sync-iobs PRIVATE src)
target_link_libraries(sync-iobs PRIVATE imgui glfw)

if(WIN32)
    # Windows 用 WinHTTP，无需第三方 HTTP 库
    target_link_libraries(sync-iobs PRIVATE winhttp)
    target_compile_definitions(sync-iobs PRIVATE _WIN32_WINNT=0x0600)
    set_target_properties(sync-iobs PROPERTIES WIN32_EXECUTABLE TRUE)
    # 子系统为 windows 时链接器找 WinMain；指定入口为 mainCRTStartup 就能沿用 main()
    target_link_options(sync-iobs PRIVATE "/ENTRY:mainCRTStartup")
else()
    # macOS / Linux 用 libcurl
    find_package(CURL REQUIRED)
    target_link_libraries(sync-iobs PRIVATE CURL::libcurl)
endif()

if(APPLE)
    target_compile_definitions(sync-iobs PRIVATE GL_SILENCE_DEPRECATION)
endif()
