# 在红魔 9 Pro+ 上安装 dev-workspace（无需 root）

本指南按 [Podroid 官方文档](https://github.com/ExTV/Podroid) 整理。app 内的按钮名称以实际界面为准；
桌面流畅度以你实测为准（我没有安卓设备，无法代你验证速度）。

> **路线**：Podroid = 免 root 的 Alpine Linux VM，自带 Docker。
> 你的机器是骁龙 8 Gen 3，非 Pixel，无 root，所以走 **QEMU 软件模拟**后端（没有硬件加速——那需要 root 或 Pixel）。

---

## 开始前

- **手机**：红魔 9 Pro+（arm64 / Android 8+ ✓，24GB/1TB 足够）。不需要 root，不需要解锁 BL。
- **电脑**：推荐直接下载已经验证的 `v1.0.0` ARM64 镜像包到家目录 `~/`（不是项目目录）：
  - `~/dev-workspace-kali-kasmvnc-linux-arm64.tar.gz`（约 1.1 GB，Kali + KasmVNC + code-server）
  - 可选的本地轻量包 `~/sdw-image.tar.gz` 只有在你按文末命令自行构建后才存在。
  - 文件不在了或代码更新过：优先按下方命令重新下载 Release；需要 headless 才本地构建。
- **网络**：手机和 Mac 连**同一个 Wi-Fi**（要传 1GB 镜像）。

### 在 Mac 下载已验证的 ARM64 镜像

仓库是私有的，先确保 GitHub CLI 已登录，然后下载并校验包结构：

```sh
cd ~
gh auth status
gh release download v1.0.0 \
  --repo crkmythical/dev-workspace \
  --pattern 'dev-workspace-kali-kasmvnc-linux-arm64.tar.gz' \
  --clobber
tar -tzf ~/dev-workspace-kali-kasmvnc-linux-arm64.tar.gz > /dev/null && echo OK
```

也可以从 `https://github.com/crkmythical/dev-workspace/releases/tag/v1.0.0` 登录后用浏览器下载。

---

## 一、装 Podroid

1. 手机浏览器打开 `https://github.com/ExTV/Podroid/releases/latest`，下载最新的 `.apk`。
2. 点开安装，按提示允许「安装未知来源应用」。
3. 打开 Podroid（界面可在设置里切成中文）。

## 二、启动虚拟机并配置

1. 首次进入有设置向导：**打开 SSH**（第三步传文件要用），其余保持默认。
2. 进设置，CPU 设为约 **4 核**（别拉满，Podroid 实测 8 核比 4 核更慢），内存给 **6–8 GB**。
3. 回主界面点 **Start VM**，等出现 **Ready!**。
4. 现在你在一个 Alpine Linux VM 里，自带 `docker`，而且你是 root。

## 三、把镜像传进 VM（最容易卡住的一步）

> ⚠️ QEMU 后端**没有**「手机文件夹共享」（那是 Pixel 的 AVF 专属功能）。
> 所以**不能**用文件管理器把 tar 拷进去，必须走网络。

1. 在 Podroid 的终端里查手机 IP：
   ```sh
   podroid-phone-ip
   ```
2. 在 Mac 终端（和手机同 Wi-Fi）传镜像，密码是 `podroid`：
   ```sh
   scp -P 9922 ~/dev-workspace-kali-kasmvnc-linux-arm64.tar.gz root@<手机IP>:/root/
   ```
   1GB 走 Wi-Fi 大约几分钟。

## 四、导入并启动工作区

在 Podroid 终端里：

```sh
docker load < /root/dev-workspace-kali-kasmvnc-linux-arm64.tar.gz

docker run -d --name dev-workspace \
  --privileged --device /dev/fuse \
  -e PASSWORD='换成你自己的登录密码' \
  -e CLASH_SUBSCRIPTION_URL='你的 clash 订阅链接' \
  -e DESKTOP_RESOLUTION=1280x720 \
  -p 8080:8080 \
  dev-workspace-kali-kasmvnc:latest
```

等容器就绪：`docker ps` 里状态出现 `(healthy)` 再去开浏览器（首次要解锁 vault、拉起桌面。Mac 上约 40 秒，
手机是软件模拟，会慢不少）。看启动日志：

```sh
docker logs -f dev-workspace
```

- `CLASH_SUBSCRIPTION_URL` 是出口代理订阅，暂时没有可以先填个占位（如 `https://example.invalid/sub`）先把界面跑通。
- `DESKTOP_RESOLUTION=1280x720` 是降分辨率减负；想更清晰可改回 `1920x1080`，但会更卡。

## 五、把界面转发到手机浏览器

在 Podroid 终端：

```sh
podroid-forward add 8080 8080 tcp
```

然后用**手机浏览器**打开 `http://localhost:8080/`：

- 登录：解锁页（Unlock workspace）只有一个密码框，输入你上面设的 `PASSWORD`，**没有用户名**
- 登录后进 `http://localhost:8080/vnc/` 看 **XFCE 桌面**
- 网页版 VS Code（code-server）在同一个入口

## 六、建议：先验证链路，再上桌面

桌面在软件模拟下最吃性能。第一次最好把「能不能跑通」和「桌面顺不顺」分开：

1. 如果你另外构建了轻量包 `~/sdw-image.tar.gz`，先传它，第四步把镜像名换成 `dev-workspace:kali-headless`、去掉 `DESKTOP_RESOLUTION`，跑通登录，证明 Podroid + Docker + 转发正常；
2. 再换 Release 中的 `dev-workspace-kali-kasmvnc:latest` 测试桌面。

这样一旦出问题，好判断是环境没装好还是单纯桌面卡。

---

## 附：下载或重新生成镜像包

默认桌面包优先使用本文开头的 `gh release download`，无需在 Mac 重建。只有需要最新未发布代码或自定义 headless 配置时，才在项目专用的 Colima VM 中构建；终端默认 Docker 是 Docker Desktop，不要用它。

```sh
cd ~/IdeaProjects/dev-workspace
export DOCKER_CONTEXT=colima-dev-workspace   # 只对当前终端生效，不改全局设置
docker context show                          # 必须输出 colima-dev-workspace
```

**与 v1.0.0 Release 相同的默认桌面配置**（输出 `~/dev-workspace-kali-kasmvnc-linux-arm64.tar.gz`）：

```sh
BASE_PROFILE=kali BASE_IMAGE=kalilinux/kali-rolling WORKSPACE_VARIANT=kali-kasmvnc \
  DESKTOP_STACK=kasmvnc EDITOR_SUITE=code-server DEV_RUNTIMES=standard \
  DESKTOP_APPS=firefox PENTEST_SUITE=bootstrap BACKUP_TRANSFER=standard make build \
  && docker save -o ~/dev-workspace-kali-kasmvnc-linux-arm64-new.tar dev-workspace:kali-kasmvnc \
  && gzip -f ~/dev-workspace-kali-kasmvnc-linux-arm64-new.tar \
  && tar -tzf ~/dev-workspace-kali-kasmvnc-linux-arm64-new.tar.gz > /dev/null \
  && mv ~/dev-workspace-kali-kasmvnc-linux-arm64-new.tar.gz \
    ~/dev-workspace-kali-kasmvnc-linux-arm64.tar.gz
```

**可选的轻量 code-server 配置** `dev-workspace:kali-headless`（输出 `~/sdw-image.tar.gz`）：

```sh
BASE_PROFILE=kali BASE_IMAGE=kalilinux/kali-rolling WORKSPACE_VARIANT=kali-headless \
  DESKTOP_STACK=none EDITOR_SUITE=code-server DEV_RUNTIMES=none DESKTOP_APPS=none \
  PENTEST_SUITE=none BACKUP_TRANSFER=none make build \
  && docker save -o ~/sdw-image-new.tar dev-workspace:kali-headless \
  && gzip -f ~/sdw-image-new.tar \
  && tar -tzf ~/sdw-image-new.tar.gz > /dev/null \
  && mv ~/sdw-image-new.tar.gz ~/sdw-image.tar.gz
```

- 耗时：取决于缓存和下载速度，几分钟到半小时不等（apt 那几层要重做、镜像站又慢时最久）；`docker save` 导出约 15 秒到 2 分钟。
- 构建走 `.env` 里的 `BUILD_PROXY`（Colima 把 `192.168.5.2:1080` 转到 Mac 本机 1080 端口的代理），代理软件要开着，否则下载步骤会失败。
- 整条命令用 `&&` 串起来：构建失败就不导出，导出失败就不替换，原来能用的文件不会被覆盖。先写 `-new` 文件，校验通过才 `mv`。
- 导出用 `docker save -o` 再 `gzip`，不要写成 `docker save ... | gzip > 文件`：管道里 `docker save` 失败（比如 `No such image`）时 gzip 照样成功，得到一个 20 字节的空包，`tar -tzf` 还会放行（退出码 0），随后 `mv` 就把好文件覆盖了。
- 校验用 `tar -tzf`，不要用 `gzip -t`：导出被截断时，`gzip -t` 仍然通过。
- Colima VM 和手机都是 arm64，镜像直接可用。
- 生成后回到「三」传镜像。

---

## 关掉 / 重来

```sh
docker rm -f dev-workspace     # 删容器
podroid-forward clean          # 清掉所有端口转发
```
VM 本身在 Podroid 主界面点 **Stop**。

## 性能预期（实话）

- 骁龙 8 Gen 3 + 无 root = **软件模拟**，拿不到硬件加速。
- 写代码、开终端、轻度浏览：可用；桌面拖大窗口、动画、看视频：会卡。
- **24GB 内存解决不了卡**——瓶颈是 CPU 单核指令翻译速度，不是内存。
- 我没在真机上实测过帧率（没有安卓设备），到底能不能接受，只有你试了才算数。

## 常见问题

- **`scp` 连不上**：确认手机和 Mac 同一 Wi-Fi、SSH 已在 Podroid 里打开、IP 用 `podroid-phone-ip` 查到的那个。
- **`scp` 报 `No such file or directory`**：镜像包在家目录，不在项目目录。用 `ls -lh ~/dev-workspace-*.tar.gz ~/sdw-image*.tar.gz 2>/dev/null` 查看；默认包不存在就重新运行本文开头的 `gh release download`。
- **`docker save` 提示 `No such image`**：没先 `make build`，或当前终端没设 `DOCKER_CONTEXT=colima-dev-workspace`（镜像在 Colima 的 Docker 里，不在 Docker Desktop 里）。
- **`make build` 卡在下载或报网络错误**：确认代理软件开着、Mac 本机 1080 端口在监听（`.env` 里的 `BUILD_PROXY` 经 Colima 转到它）。
- **`make build` 在 `fetch-tool` 一步报 `curl: (35) TLS connect error ... unexpected eof`**：GitHub 下载被中途断开，偶发。直接重跑 `make build`，已完成的层有缓存，会从失败的那一步继续。
- **提示找不到 `/dev/fuse`**：说明该 VM 没有 fuse 设备节点；vault 依赖它。先确认 Podroid 为最新版本。
- **浏览器打不开 `localhost:8080`**：确认 `docker ps` 里容器在跑、`podroid-forward add` 执行过。
- **桌面太卡**：分辨率降到 `1280x720` 或更低；日常尽量用 code-server，只在需要 GUI 工具时开 `/vnc/`。
