# CMVS Switch · NSP 封包与单游戏直启说明

这份文档对应 `scripts/pack-nsp.sh`（Git Bash 脚本）和 `scripts/pack-nsp.bat`（双击/拖拽入口）。
封包流程严格遵守一份经过真机反复实测的 NSP 打包手册（下称《手册》，不随本仓库分发）——
本脚本不引入任何《手册》之外的"自创"参数。

> **自备工具**（见第 2 节环境变量）：你自己的 `prod.keys`（从自己的机器 dump）、
> 黄金参照 `main.npdm`、hacBrewPack 可执行文件、`pack_check.py` 与
> `patch_npdm_titleid.py` 所在的工具目录。这些均不随本仓库分发。

> **前提：你的 Switch 必须已安装 sigpatches。**
> 本流程打出的 NSP 和黄金参照一样，NCA 签名是固定占位数据（无 ticket/cert 的 3 文件包）。
> 没有 sigpatches 的机器装得上但启动不了，这不是打包错误。

---

## 1. 两种直启模式（启动优先级）

播放器启动时按下面的顺序判断，命中即止：

| 优先级 | 模式 | RomFS 里有什么 | 行为 |
|---|---|---|---|
| 1 | **A：数据随包** | 游戏自己的文件（`data/pack/start.ps3` 或 `start.ps2` + `script.cpz`） | 直接以 `romfs:/` 为只读游戏根启动该游戏 |
| 2 | **B：指向 SD 卡** | 只有一个 `target.txt`，内容是文件夹名 | 启动 `sdmc:/switch/cmvs/<文件夹名>/` 里的游戏 |
| 3 | 都没有 | （普通 .nro，无 RomFS） | 进入原来的多游戏菜单，行为不变 |

两种模式的**存档都写在 SD 卡**：`sdmc:/switch/cmvs/users/<游戏ID>/`。
游戏 ID 来自引擎注册表对游戏数据的识别，不是文件夹名——所以模式 A 即使根目录叫
`romfs`，不同游戏的存档也不会串。模式 A 的游戏数据是只读的，游戏不会往包里写东西。

模式 A/B 怎么选：

- **模式 A**：一个 NSP 装完即玩，SD 卡上不需要游戏数据。缺点是 NSP 和游戏数据一样大
  （几 GB 的游戏就是几 GB 的 NSP，FAT32 单文件上限 4 GiB，超过就装不了）。
- **模式 B**：NSP 只有几百 KB，游戏数据照常放在 `sdmc:/switch/cmvs/<游戏名>/`。
  适合大数据游戏、或者想和 .nro 共用同一份数据/存档的情况。
  `target.txt` 由脚本自动生成（UTF-8，内容就是文件夹名），不用手写。

`target.txt` 的安全校验（播放器侧）：只允许一个文件夹名，含 `/`、`\`、`.`、`..`
或指向不存在的文件夹都会报错并显示原因，不会误启动别的目录。

## 2. 一键封包

### 最简单用法（模式 B，推荐先试这个）

把游戏文件夹**拖到** `scripts\pack-nsp.bat` 上，或在终端里：

```bat
pack-nsp.bat "D:\Games\MyGame\解压后的游戏目录" --mode b --title "游戏标题"
```

### Git Bash 用法

```sh
# 模式 B：小 NSP，数据留在 SD 卡
sh platforms/switch/scripts/pack-nsp.sh "/d/Games/MyGame/游戏目录" \
    --mode b --title "游戏标题"

# 模式 A：游戏数据整个打进 NSP（要求目录里有 data/pack/start.ps3|ps2 + script.cpz）
sh platforms/switch/scripts/pack-nsp.sh "/d/Games/MyGame/游戏目录" --mode a --title "游戏标题"
```

### 需要自备的工具（环境变量）

脚本不含任何密钥/打包工具，运行前先设置（Git Bash）：

```sh
export CMVS_KEYS="/path/to/prod.keys"          # 你自己机器 dump 的密钥（勿分发）
export CMVS_GOLDEN_NPDM="/path/to/main.npdm"   # 硬件验证过的黄金参照 NPDM
export CMVS_HACBREWPACK="/path/to/hacbrewpack" # hacBrewPack 可执行文件
export CMVS_HANDOVER="/path/to/tool-dir"       # 含 pack_check.py 与 patch_npdm_titleid.py
```

可选覆盖：`DEVKITPRO`、`CARGO_TARGET_DIR`、`CMVS_PACK_DIR`（必须纯 ASCII）、
`CMVS_PYTHON`（默认取 PATH 上的 `python`，需要 Pillow）、`--icon` 默认图标等。
`prod.keys` 属于任天堂版权密钥，请自行 dump、绝不随仓库分发。

常用选项：

| 选项 | 默认 | 说明 |
|---|---|---|
| `--mode a\|b` | `a` | 见上表 |
| `--title "标题"` | 游戏文件夹名 | 主界面显示的名字（UTF-8，中日文都可以） |
| `--author "作者"` | `cmvs-rs` | 主界面显示的发行商 |
| `--titleid HEX` | `0100C65000030000` | 见第 4 节 |
| `--icon 图片` | `build/icon-default.jpg` | 任意尺寸 PNG/JPEG，脚本自动转 256×256 JPEG |
| `--version X.Y.Z` | `1.0.0` | 显示版本 |
| `--no-build` | 关 | 跳过重新编译，直接用上次构建的 ELF（改过代码就别用） |
| `--out 目录` | `platforms/switch/build/nsp` | 成品 NSP 复制到哪里 |

产物命名：`<标题>[<TitleID>][v0].nsp`。用 DBI / Tinfoil / Awoo 安装即可。

### 每游戏按键与字体

- `buttons.txt` 照旧放在**游戏根目录**（模式 A 会被打进包里，只读生效；
  模式 B 放在 SD 卡的游戏文件夹里）。快捷键默认：X=存档画面、Y=读档画面、
  ZL+ZR=系统设置、L+X / L+Y=快速存/读档（860 槽，与游戏内 quick save 互通）。
  详见 `build/buttons.txt.example`。
- 自定义字体 `font.ttf`：先找 `sdmc:/switch/cmvs/font.ttf`，再找 `romfs:/font.ttf`
  （模式 A 可随包携带），都没有就用系统共享字体。

## 3. 脚本做了什么（与《手册》逐条对应）

1. **重新编译**播放器（`build-player-win.sh`，同时刷新 .nro），NSO 与 .nro 来自同一份 ELF。
2. **ASCII 工作目录** `C:\cmvs-nsp-pack`（可用 `CMVS_PACK_DIR` 改，但必须纯 ASCII——
   hacBrewPack 处理不了中文路径）。模式 A 的 RomFS 用 **junction** 指向真实游戏目录，
   不复制几 GB 数据；junction 本身在 ASCII 路径上。
3. **ExeFS**：`elf2nso` 生成 `main`；`main.npdm` 取**黄金参照**
   （`CMVS_GOLDEN_NPDM` 指向的 main.npdm，脚本先校验 sha256 前缀 `bbbc0e6d7f154ab2`），
   用《手册》的 `patch_npdm_titleid.py` **只改 title id 三处**（ACI0+0x10、ACID+0x10/+0x18），
   flags/SAC/kernel caps 一个字节都不动。
4. **Control**：`nacptool --create` 先用 ASCII 占位生成（devkitPro 工具的 argv 可能是
   ANSI，中文标题直接传会乱码），再用本仓库的 `scripts/patch_nacp.py` 以 UTF-8 写入
   全部 16 个语言槽，并按黄金值设置：startup_user_account=1、语言位图 0x0BFF、
   存档 62 MB/1.5 MB、设备存档 0、显示版本无前导空格。
   图标：`icon.jpg` + `icon_AmericanEnglish.dat`，都是 256×256 JPEG。
5. **hacBrewPack** 打 3 文件 NSP（program/meta/control，无 ticket/cert）。
6. **体检门禁**：`pack_check.py --romfs` 必须 **0 硬失败**，否则脚本直接报错、
   不会把 NSP 复制出来。软警告里"ACID 范围收窄到单个 title id"是预期内的
   （patch_npdm_titleid.py 的正常结果），其余软警告请对照《手册》第 2 节逐条看。

## 4. TitleID 怎么选

- 必须是 `0100XXXXXXXX0000` 形状（应用号段；`…0800` 是更新、`…1000` 是 DLC，别用）。
  脚本会拒绝不合形状的值。**"05 开头更安全"是错的**（《手册》§3.2）。
- 不要撞真实游戏：撞了会被官方更新覆盖、存档归属串掉。默认值 `0100C65000030000`
  是自建号段；给不同游戏封包时建议各用一个不同的随机中段，例如
  `0100C65000030000`、`0100C65100030000`……（用 `--titleid` 传入）。
- 同一个游戏重新封包（改标题/图标/版本）请**沿用同一个 TitleID**，
  这样系统把它当同一个程序的覆盖安装。

## 5. 已验证 / 未验证（诚实清单）

已在本机验证：

- 播放器全部宿主单元测试通过（含模式 A/B 启动判定、target.txt 校验的 4 个新测试）。
- 封包流水线冒烟测试：模式 B（中文文件夹名/标题）与模式 A（junction）各打出 NSP，
  `pack_check.py` 0 硬失败（冒烟用数据是占位文件，**不是可玩的游戏包**）。

**尚未验证（需要你的实机/模拟器）**：

- NSP 在真机上的安装与启动（模式 A 的 `romfs:/` 只读根、模式 B 的 target.txt 直启）。
- 底座/掌机切换、快捷键、快速存读档在 NSP 模式下的表现（逻辑与 .nro 完全相同，
  但 NSP 启动路径本身没上过机）。
- 模拟器（Eden/Ryujinx）能启动**不能**证明 NPDM 正确——《手册》反复强调过这一点；
  真机报错时请优先取 `atmosphere/crash_reports/` 里的报告。

## 6. 症状速查（摘自《手册》§7）

| 症状 | 最可能原因 | 怎么办 |
|---|---|---|
| 启动立刻 2168-0008（Svc 0x28 / X0=0xf801） | NPDM 缺 SVC 白名单 | 本脚本用黄金 npdm，正常不会出；出了先确认没换过 npdm 来源 |
| 装得上、启动一闪而过 / 2155-8007 | 没装 sigpatches，或 title id 不一致 | 先确认 sigpatches；再跑 `pack_check.py` |
| 2002-3006 数据损坏 | RomFS 哈希树/大小错 | `pack_check.py <nsp> --full-ivfc` 全量校验 |
| 模式 B 报 "target.txt names … does not exist" | SD 卡上没有那个文件夹 | 把游戏数据放到 `sdmc:/switch/cmvs/<文件夹名>/` |
| 主界面名字乱码 | （本流程已防）NACP 非 UTF-8 | 用本脚本重新封包，不要手搓 nacp |
| 打包工具崩 / 找不到文件 | 工作目录含中文 | 保持 `CMVS_PACK_DIR` 为纯 ASCII |

**不要做的事**（《手册》§8 原文结论）：不要从别的移植整体拷 npdm；不要手改
npdm 的 flags/SAC/kernel caps；不要为"补票据"重打 titlekey 版；不要去"修"
CNMT digest=0 / required_system_version=0（黄金参照也这样）；不要在中文路径下跑打包器。