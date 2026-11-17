# CMVS Switch 二次移植增强（hbmenu 启动器 + 每游戏 NSP 直启）

本文档记录在[上游 Switch 播放器](README.md)（见 `git log` 中 `Merge pull request #14` 之前的版本，
基线提交 `34fdb7e`）之上的一次移植增强，由 @postskt 于 2026-11 完成。
按 **Apache-2.0 §4(b)** 要求声明：本次改动涉及的所有修改文件均已在 git 历史中标注。

## 与上游的分工

- **上游已有**：Switch 播放器本体（deko3d GPU/软件双渲染、多游戏列表、编码检测、
  系统字体、触摸、软键盘、错误页等），见 [README.md](README.md)。
- **本次增强**：多游戏通用启动器路径、每游戏参数化按键、游戏内快捷键与 OSD 提示、
  单游戏 NSP 直启（模式 A/B）、启动动画跳过修复、Windows 无 Docker 构建与 NSP 一键封包工具链。

## 新增/修改文件

| 文件 | 说明 |
|---|---|
| `player/src/buttons.rs`（新增） | 每游戏 `buttons.txt` 参数化按键映射（游戏内动作 + 工具栏目标） |
| `player/src/hotkeys.rs`（新增） | 组合键热键轮询：存档/读档/设置、快速存/读档 |
| `player/src/toast.rs`（新增） | OSD 提示（快速存/读档反馈） |
| `player/src/main.rs` | 主循环接线：SD 路径 `sdmc:/switch/cmvs/<游戏>/`、热键分发、工具栏自动点击、电影期间快进注入 |
| `player/src/gpu.rs`、`runtime/gpu.c`、`runtime/window.c` | 底座 1080p / 掌机 720p 动态切换等显示适配 |
| `crates/cmvs-native/src/switch/{gpu_device,window}.rs` | 显示模式枚举与窗口尺寸联动 |
| `crates/cmvs-console/src/{controls,library}.rs` | 控制台能力、多游戏库路径 |
| `crates/cmvs-probe/src/bin/op-skip.rs`（新增） | 无头探针：复现/验证"启动动画跳过"修复 |
| `scripts/build-player-win.sh`（新增） | Windows/Git-Bash 免 Docker 交叉构建（devkitA64 → ELF → elf2nro） |
| `scripts/pack-nsp.sh`、`pack-nsp.bat`（新增） | 一键 NSP 封包，见 [NSP-PACKING.md](NSP-PACKING.md) |
| `scripts/patch_nacp.py`（新增） | NACP 16 语言槽 UTF-8 标题写入 + 黄金参照字段 |
| `NSP-PACKING.md`（新增） | 封包手册（自备工具清单、TitleID 规则、症状速查） |

## 三种启动形态

| 形态 | 文件 | 说明 |
|---|---|---|
| 通用启动器 NRO | `cmvs-switch.nro` | hbmenu 启动，读 `sdmc:/switch/cmvs/<游戏名>/`，多游戏共用 |
| 单游戏内嵌字体 NRO | `cmvs-switch-<游戏>.nro` | RomFS 内嵌 `font.ttf`（绕过模拟器 HLE romfs 字体缺陷），hbmenu 直启 |
| 单游戏 NSP 直启 | `<标题>[<titleid>][v0].nsp` | 模式 B（romfs 只放 `target.txt`），安装后像正版游戏一样直启，**需 sigpatches** |

模式 A（游戏数据打进 romfs 的大 NSP）与模式 B（小 NSP + SD 卡数据）详见
[NSP-PACKING.md](NSP-PACKING.md)。两种模式存档统一写在 SD 卡
`sdmc:/switch/cmvs/users/<游戏ID>/`，游戏目录保持只读。

## 默认按键映射（`buttons.txt` 可改）

| 按键 | 职责 |
|---|---|
| A / B | 决定（鼠标左键）/ 取消（鼠标右键） |
| X / Y | 存档画面 / 读档画面（自动点击游戏工具栏对应按钮） |
| ZL+ZR | 系统设置（自动点击游戏工具栏） |
| L+X / L+Y | 快速存档 / 快速读档（主机快照 860 槽，与游戏自带 quick save 互通） |
| L / − | 回想日志滚动（滚轮上/下） |
| ZR / R / ZL | 快进（单按延迟 0.12 秒生效；组合键优先） |
| 右摇杆 | 光标微调 |
| +（长按 1 秒） | 退出游戏 |

示例文件见 `build/buttons.txt.example`。

## 启动动画"跳不过、反复播放"修复记录

- **"反复播放"是原版 attract 循环的正常设计**（标题待机 → 自动播 OP → 回标题）。
- **"跳不过"根因**：op 电影等待循环只查决定/取消键的按下边沿，不查快进标志——
  快进键（ZR/R/ZL）按住无效，只有 A/B 能跳。
- **修复**（最小改动，仅 Switch 播放器 `main.rs`，不动共享引擎与游戏脚本）：
  电影播放期间若按住快进键，向引擎注入一次"决定"按下，脚本消费后停播；
  电影一停注入即止，快进键恢复原职责。无头探针 `op-skip.rs` 验证 2 帧内停播。

## 构建与封包

```sh
# 交叉构建 NRO（Windows Git Bash，无需 Docker）
sh platforms/switch/scripts/build-player-win.sh

# 一键 NSP 封包（自备 keys/黄金 NPDM/hacBrewPack，见 NSP-PACKING.md）
export CMVS_KEYS=... CMVS_GOLDEN_NPDM=... CMVS_HACBREWPACK=... CMVS_HANDOVER=...
sh platforms/switch/scripts/pack-nsp.sh <游戏目录> --mode b --title "游戏标题"
```

桌面版构建见[运行指南](../guides/running.md)。

## 验证状态（截至 2026-11-18）

- ✅ Ryujinx 1.3.3 严格环境完整跑通：标题 → 开始 → 对话 → 快速存/读档 → NSP 直启（60 FPS）。
- ✅ NSP 直启（真实固件 dump + gen16 密钥）、固件共享字体 pl:u 路径、启动动画跳过（无头验证）。
- ⏳ 待真机：触摸、音频输出、底座/掌机实机切换、FAT32/exFAT 实卡、Atmosphere 下 NSP 安装。

## 许可与法律边界

- 本目录新增代码沿用仓库的 [MIT](../../LICENSE-MIT) OR [Apache-2.0](../../LICENSE-APACHE) 双许可。
- 内嵌/随包字体（如 BIZ UDPGothic）按其许可证分发（SIL OFL 要求附带许可证文本）。
- **游戏数据不属于本仓库**：`data/pack/*.cpz`、`*.cmv` 等商业游戏资源请自备正版，
  `.gitignore` 已排除全部游戏资源格式。
- **密钥与 NSP 成品不入仓库**：`prod.keys` 等任天堂密钥请自行 dump；
  NSP 请用脚本自行封包，本仓库不提供成品 NSP。
