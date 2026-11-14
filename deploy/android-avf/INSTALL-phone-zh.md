# 在红魔 9 Pro+ 上安装 dev-workspace（无需 root）

本指南按 [Podroid 官方文档](https://github.com/ExTV/Podroid) 整理。app 内的按钮名称以实际界面为准；
桌面流畅度以你实测为准（我没有安卓设备，无法代你验证速度）。

> **路线**：Podroid = 免 root 的 Alpine Linux VM，自带 Docker。
> 你的机器是骁龙 8 Gen 3，非 Pixel，无 root，所以走 **QEMU 软件模拟**后端（没有硬件加速——那需要 root 或 Pixel）。

---

## 开始前

- **手机**：红魔 9 Pro+（arm64 / Android 8+ ✓，24GB/1TB 足够）。不需要 root，不需要解锁 BL。
- **电脑**：你的 Mac，镜像包已经打好：
  - `~/sdw-image-gui.tar.gz`（1.0G，**带桌面**，跑 GUI 用这个）
  - `~/sdw-image.tar.gz`（875M，只有 code-server，先跑通验证用这个）
- **网络**：手机和 Mac 连**同一个 Wi-Fi**（要传 1GB 镜像）。

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
   scp -P 9922 ~/sdw-image-gui.tar.gz root@<手机IP>:/root/
   ```
   1GB 走 Wi-Fi 大约几分钟。

## 四、导入并启动工作区

在 Podroid 终端里：

```sh
docker load < /root/sdw-image-gui.tar.gz

docker run -d --name dev-workspace \
  --privileged --device /dev/fuse \
  -e PASSWORD='换成你自己的登录密码' \
  -e CLASH_SUBSCRIPTION_URL='你的 clash 订阅链接' \
  -e DESKTOP_RESOLUTION=1280x720 \
  -p 8080:8080 \
  dev-workspace:kali
```

等十几秒让它起来（首次要解锁 vault、拉起桌面）。看启动日志：

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

- 登录：用户名 `user`，密码 = 你上面设的 `PASSWORD`
- 登录后进 `http://localhost:8080/vnc/` 看 **XFCE 桌面**
- 网页版 VS Code（code-server）在同一个入口

## 六、建议：先验证链路，再上桌面

桌面在软件模拟下最吃性能。第一次最好把「能不能跑通」和「桌面顺不顺」分开：

1. 先传轻量镜像 `~/sdw-image.tar.gz`，第四步把镜像名换成 `dev-workspace:headless`、去掉 `DESKTOP_RESOLUTION` 那行，跑通登录 → 证明 Podroid + Docker + 转发都正常；
2. 再换 GUI 镜像 `dev-workspace:kali` 试桌面。

这样一旦出问题，好判断是环境没装好还是单纯桌面卡。

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
- **提示找不到 `/dev/fuse`**：说明该 VM 没有 fuse 设备节点；vault 依赖它。先确认 Podroid 为最新版本。
- **浏览器打不开 `localhost:8080`**：确认 `docker ps` 里容器在跑、`podroid-forward add` 执行过。
- **桌面太卡**：分辨率降到 `1280x720` 或更低；日常尽量用 code-server，只在需要 GUI 工具时开 `/vnc/`。
