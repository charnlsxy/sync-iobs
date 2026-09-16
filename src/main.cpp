// sync-iobs —— C++ / Dear ImGui 版
//
// 骨架阶段目标：验证 CMake + Dear ImGui + GLFW 在 Windows / macOS / Linux 三平台
// 都能编译通过。功能逻辑（token 签名、上传下载、配置解析）随后逐步移植过来。

#include "imgui.h"
#include "imgui_impl_glfw.h"
#include "imgui_impl_opengl3.h"
#include "imgui_impl_opengl3_loader.h"

#include <GLFW/glfw3.h>

#include <cstdio>
#include <string>
#include <vector>

#ifdef _WIN32
#include <windows.h>
#endif

namespace {

// 各平台的中文字体候选。Dear ImGui 内置字体不含中文，不挂系统字体会全是方块。
// 注意 macOS 的中文字体多为 .ttc（字体集合），需通过 ImFontConfig::FontNo 选字面。
struct FontCandidate {
    const char* path;
    int font_no;
};

std::vector<FontCandidate> font_candidates() {
    std::vector<FontCandidate> c;
#ifdef _WIN32
    c.push_back({"C:\\Windows\\Fonts\\msyh.ttc", 0});
    c.push_back({"C:\\Windows\\Fonts\\simhei.ttf", 0});
    c.push_back({"C:\\Windows\\Fonts\\Deng.ttf", 0});
#elif defined(__APPLE__)
    c.push_back({"/System/Library/Fonts/PingFang.ttc", 0});
    c.push_back({"/System/Library/Fonts/STHeiti Light.ttc", 0});
    c.push_back({"/System/Library/Fonts/Supplemental/Songti.ttc", 0});
#else
    c.push_back({"/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0});
    c.push_back({"/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0});
#endif
    return c;
}

void setup_fonts() {
    ImGuiIO& io = ImGui::GetIO();
    // 先挂一份内置字体做兜底，再叠加中文字体
    io.Fonts->AddFontDefault();
    for (const auto& cand : font_candidates()) {
        ImFontConfig cfg;
        cfg.FontNo = cand.font_no;
        cfg.MergeMode = true;          // 中文字形合并进默认字体
        cfg.OversampleH = 2;
        cfg.OversampleV = 2;
        ImFont* f = io.Fonts->AddFontFromFileTTF(
            cand.path, 16.0f, &cfg, io.Fonts->GetGlyphRangesChineseFull());
        if (f) {
            std::printf("[font] %s\n", cand.path);
            return;
        }
    }
    std::printf("[font] no CJK font found, Chinese may render as boxes\n");
}

}  // namespace

int main() {
    if (!glfwInit()) {
        std::fprintf(stderr, "glfwInit failed\n");
        return 1;
    }

    // OpenGL 3.3 core：macOS 必须为 forward-compatible
    glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);
    glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 3);
    glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);
#ifdef __APPLE__
    glfwWindowHint(GLFW_OPENGL_FORWARD_COMPAT, GLFW_TRUE);
#endif

    GLFWwindow* window = glfwCreateWindow(900, 660, "sync-iobs", nullptr, nullptr);
    if (!window) {
        std::fprintf(stderr, "glfwCreateWindow failed\n");
        glfwTerminate();
        return 1;
    }
    glfwMakeContextCurrent(window);
    glfwSwapInterval(1);  // 开启垂直同步，避免空转烧 CPU

    IMGUI_CHECKVERSION();
    ImGui::CreateContext();
    ImGuiIO& io = ImGui::GetIO();

    setup_fonts();

    ImGui::StyleColorsLight();
    ImGuiStyle& style = ImGui::GetStyle();
    style.WindowRounding = 6.0f;
    style.FrameRounding = 4.0f;
    style.ItemSpacing = ImVec2(10.0f, 10.0f);
    style.FramePadding = ImVec2(10.0f, 5.0f);

    ImGui_ImplGlfw_InitForOpenGL(window, true);
    ImGui_ImplOpenGL3_Init("#version 330");

    while (!glfwWindowShouldClose(window)) {
        glfwPollEvents();

        ImGui_ImplOpenGL3_NewFrame();
        ImGui_ImplGlfw_NewFrame();
        ImGui::NewFrame();

        // 铺满整窗的宿主窗口，模拟原 egui 版的单窗口布局
        ImGui::SetNextWindowPos(ImVec2(0.0f, 0.0f));
        ImGui::SetNextWindowSize(io.DisplaySize);
        ImGui::Begin("sync-iobs", nullptr,
                     ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoMove |
                         ImGuiWindowFlags_NoResize | ImGuiWindowFlags_NoBringToFrontOnFocus);

        ImGui::Text("sync-iobs");
        ImGui::SameLine();
        ImGui::TextDisabled("iobs 文件双向同步");
        ImGui::Separator();
        ImGui::TextWrapped(
            "骨架阶段：验证 CMake + Dear ImGui + GLFW 三平台可编译运行。"
            "功能逻辑随后移植。");
        ImGui::Spacing();
        ImGui::Text("FPS: %.1f", io.Framerate);
        ImGui::Text("中文字体测试：连接凭据 / 上传 / 下载 / 任务 / 操作日志");
        ImGui::End();

        ImGui::Render();
        int display_w = 0, display_h = 0;
        glfwGetFramebufferSize(window, &display_w, &display_h);
        glViewport(0, 0, display_w, display_h);
        glClearColor(0.94f, 0.94f, 0.95f, 1.0f);
        glClear(GL_COLOR_BUFFER_BIT);
        ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
        glfwSwapBuffers(window);
    }

    ImGui_ImplOpenGL3_Shutdown();
    ImGui_ImplGlfw_Shutdown();
    ImGui::DestroyContext();
    glfwDestroyWindow(window);
    glfwTerminate();
    return 0;
}
