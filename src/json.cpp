#include "json.hpp"

#include <cstdlib>

namespace siobs {
namespace {

struct Parser {
    const std::string& s;
    std::size_t i = 0;

    explicit Parser(const std::string& text) : s(text) {}

    void skip_ws() {
        while (i < s.size() && (s[i] == ' ' || s[i] == '\t' || s[i] == '\n' || s[i] == '\r')) {
            ++i;
        }
    }

    bool expect(char c) {
        skip_ws();
        if (i < s.size() && s[i] == c) {
            ++i;
            return true;
        }
        return false;
    }

    /// 解析带引号的字符串，返回 false 表示失败
    bool parse_string(std::string& out) {
        skip_ws();
        if (i >= s.size() || s[i] != '"') return false;
        ++i;
        out.clear();
        while (i < s.size() && s[i] != '"') {
            if (s[i] == '\\' && i + 1 < s.size()) {
                char e = s[i + 1];
                switch (e) {
                    case 'n': out.push_back('\n'); break;
                    case 't': out.push_back('\t'); break;
                    case 'r': out.push_back('\r'); break;
                    case 'b': out.push_back('\b'); break;
                    case 'f': out.push_back('\f'); break;
                    case 'u': {
                        // 只处理 BMP，够用；非法则原样跳过
                        if (i + 5 < s.size()) {
                            std::string hex = s.substr(i + 2, 4);
                            unsigned code = static_cast<unsigned>(std::strtoul(hex.c_str(), nullptr, 16));
                            if (code < 0x80) {
                                out.push_back(static_cast<char>(code));
                            } else if (code < 0x800) {
                                out.push_back(static_cast<char>(0xC0 | (code >> 6)));
                                out.push_back(static_cast<char>(0x80 | (code & 0x3F)));
                            } else {
                                out.push_back(static_cast<char>(0xE0 | (code >> 12)));
                                out.push_back(static_cast<char>(0x80 | ((code >> 6) & 0x3F)));
                                out.push_back(static_cast<char>(0x80 | (code & 0x3F)));
                            }
                            i += 4;
                        }
                        break;
                    }
                    default: out.push_back(e); break;
                }
                i += 2;
                continue;
            }
            out.push_back(s[i]);
            ++i;
        }
        if (i >= s.size()) return false;
        ++i;  // 跳过收尾引号
        return true;
    }

    bool parse_number(std::uint64_t& out) {
        skip_ws();
        std::size_t start = i;
        while (i < s.size() && ((s[i] >= '0' && s[i] <= '9') || s[i] == '-' || s[i] == '.')) {
            ++i;
        }
        if (start == i) return false;
        out = std::strtoull(s.substr(start, i - start).c_str(), nullptr, 10);
        return true;
    }
};

}  // namespace

std::string json_escape(const std::string& s) {
    std::string out;
    out.reserve(s.size() + 8);
    for (unsigned char c : s) {
        switch (c) {
            case '"': out += "\\\""; break;
            case '\\': out += "\\\\"; break;
            case '\n': out += "\\n"; break;
            case '\r': out += "\\r"; break;
            case '\t': out += "\\t"; break;
            default:
                if (c < 0x20) {
                    char buf[8];
                    std::snprintf(buf, sizeof(buf), "\\u%04x", c);
                    out += buf;
                } else {
                    out.push_back(static_cast<char>(c));
                }
        }
    }
    return out;
}

std::vector<HistoryItem> parse_history(const std::string& text) {
    std::vector<HistoryItem> items;
    Parser p(text);
    if (!p.expect('[')) return items;
    p.skip_ws();
    if (p.i < p.s.size() && p.s[p.i] == ']') return items;

    while (p.i < p.s.size()) {
        if (!p.expect('{')) break;
        HistoryItem item;
        while (p.i < p.s.size()) {
            p.skip_ws();
            if (p.i < p.s.size() && p.s[p.i] == '}') {
                ++p.i;
                break;
            }
            std::string field;
            if (!p.parse_string(field)) break;
            if (!p.expect(':')) break;
            p.skip_ws();
            if (p.i < p.s.size() && p.s[p.i] == '"') {
                std::string value;
                if (!p.parse_string(value)) break;
                if (field == "name") item.name = value;
                else if (field == "key") item.key = value;
            } else {
                std::uint64_t value = 0;
                if (!p.parse_number(value)) break;
                if (field == "time") item.time = value;
            }
            p.skip_ws();
            if (p.i < p.s.size() && p.s[p.i] == ',') ++p.i;
        }
        items.push_back(item);

        p.skip_ws();
        if (p.i < p.s.size() && p.s[p.i] == ',') {
            ++p.i;
            continue;
        }
        break;
    }
    return items;
}

std::string serialize_history(const std::vector<HistoryItem>& items) {
    std::string out = "[";
    for (std::size_t i = 0; i < items.size(); ++i) {
        if (i) out += ",";
        out += "{\"name\":\"" + json_escape(items[i].name) + "\",\"key\":\"" +
               json_escape(items[i].key) + "\",\"time\":" + std::to_string(items[i].time) + "}";
    }
    out += "]";
    return out;
}

std::string json_get_string(const std::string& text, const std::string& field) {
    Parser p(text);
    if (!p.expect('{')) return std::string();
    while (p.i < p.s.size()) {
        p.skip_ws();
        if (p.i < p.s.size() && p.s[p.i] == '}') break;
        std::string key;
        if (!p.parse_string(key)) break;
        if (!p.expect(':')) break;
        p.skip_ws();
        if (p.i < p.s.size() && p.s[p.i] == '"') {
            std::string value;
            if (!p.parse_string(value)) break;
            if (key == field) return value;
        } else {
            std::uint64_t dummy = 0;
            if (!p.parse_number(dummy)) break;
        }
        p.skip_ws();
        if (p.i < p.s.size() && p.s[p.i] == ',') ++p.i;
    }
    return std::string();
}

}  // namespace siobs
