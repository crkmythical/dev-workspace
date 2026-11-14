# 设计：内存驻留的 vault 密钥与硬件信任根

> **2026-11 更新：Track A 的大半已经由别的改动做掉了，B 和 C 仍然只是设计。** 登录改由 Caddy 前面的
> 会话网关一处完成，code-server（`--auth none`）和 KasmVNC（`-DisableBasicAuth`）不再有各自的口令，
> 所以磁盘上不再有登录明文（下面表里的 #3、#5 已消失）；网关只留 bcrypt 哈希。同步密钥的来源
> 从 vault 里搬到了 root 才能读的 `/run/sdw/sync-passphrase`（#7），工作区里运行的一切都是没有密钥、
> 没有 capability 的 `dev` 用户，读不到 #1–#8 里任何一处。**没有变**的是：vault 密钥仍然等于 `PASSWORD`
> （一个 secret、两种角色里的「密钥」那一半），它仍在 `.env` / 容器环境里（除非用 `PASSWORD_FILE`），
> 仍在解锁期间的 gocryptfs 进程内存里，仍可被容器里的 root 读到。下面保留原文，以便对照。
>
> **原状态：设计，未实现，未本地 PoC。** 这台开发机（macOS + colima）没有 Linux 容器可见的
> TPM2（Apple 的 Secure Enclave 不以 TPM2 形式暴露给容器），colima 的 QEMU 层也不向容器透传
> vTPM，更没有嵌套虚拟化可用来跑 microVM。因此 Track B / C 只能是设计 + 一个在真实硬件或 CI
> 上才跑得起来的验证脚本（`src/tests/smoke/manual/tpm-seal-verify.sh`），不是本地能复现的 PoC。
> Track A 是纯软件改动，本地可做，但它改变 UX 与解锁行为，留作决策项而非盲改上线。
>
> 这一项之所以是「设计而非代码」，是刻意的：盲目上一个看着更安全、实则没动密钥的改动
> （见下面「为什么只给 code-server 加 hashed-password 是装样子」），比不做更坏。

## 目标

把 vault 的加密密钥变成**内存驻留 + 硬件信任根**：静止时不以明文存在于任何磁盘、环境变量或
日志里；运行时即便容器被攻陷，读取到解锁态密钥也要跨越一道比 namespace 更硬的边界。

## 现状（实测，grounded）

主密码是**一个 secret、两种角色**：

- **登录口令（verifier）**：code-server、KasmVNC（`/vnc/`）、Caddy（`/desktop/` basic_auth）。
- **vault 密钥（key）**：gocryptfs 的 passphrase，以及同步 AEAD 密钥（PBKDF2-HMAC-SHA256 从它派生）。

**当时**它以可恢复的形式存在于这些地方（括号里是 2026-11 的现状）：

| # | 位置 | 形式 | 谁能读到 |
|---|------|------|----------|
| 1 | `.env` 的 `PASSWORD` | 明文（现状不变；`PASSWORD_FILE` 可改为文件） | 宿主磁盘 |
| 2 | 容器环境（`env_file: .env`） | 明文（现状：只有 PID 1 和 root 进程的环境里有；`dev` 读不到；用 `PASSWORD_FILE` 时环境里只有路径） | `/proc/1/environ`、`docker inspect` |
| 3 | code-server `/root/.config/code-server/config.yaml` | **明文** `password:`（**已消失**：`--auth none`） | 容器文件系统 |
| 4 | 启动日志 / 横幅 | 明文，**仅当 PASSWORD 留空、口令是生成的**（`.env` 来的永不进日志，见 `banner.rs`；生成的另存 `/var/lib/sdw/master-password`，root 0600） | `docker logs` |
| 5 | KasmVNC `$VNC_HOME/.kasmpasswd` | hash（**已消失**：`-DisableBasicAuth`，冒烟测试断言没有这个文件） | 容器文件系统 |
| 6 | gocryptfs 进程内存 | 派生主密钥（现状不变） | 解锁期间，容器内 root |
| 7 | vault 内 `/workspace/.credentials/sync-passphrase` | 明文（**已搬走**：`/run/sdw/sync-passphrase`，root 才能读，vault 外，`lock-vault` 删除） | 解锁期间 |

（gocryptfs 磁盘上的 `gocryptfs.conf` 存的是 scrypt 包裹的 master-key 校验，不是 passphrase 本身；
所以 #6 是内存里的风险，不是静止明文。）

## 为什么只给 code-server 加 hashed-password 是装样子

code-server 支持用 `hashed-password:` 代替明文 `password:`。只做这一步看似消掉了 #3，实则没动密钥：

- **同一个字符串**仍在 #1/#2 以明文存在——这是设计使然，无人值守重启要靠 `PASSWORD` 自动解锁。
- 解锁时它仍以**明文**喂给 gocryptfs（#6 的来源）。
- 它仍以**明文**喂给 `kasmvncpasswd`（#5）。
- 而且 `desktop_start.rs` 的 `master_password_from_config()` 现在正是**从 config.yaml 把明文读回来**
  再喂给 KasmVNC——把 config.yaml 换成 hash，这条路径反而断了，除非明文另有来源。

结论：只哈希一份拷贝，密钥照样明文躺在好几处，只是**看起来**更安全。这是典型的 false-assurance，
不做。真正要解决的是「登录口令」和「vault 密钥」被当成同一个东西。

## 真实设计（三条线）

### Track A — 把两种角色拆开（纯软件，本地可做，留作决策）

- **登录口令**只以 verifier hash 落地：code-server 用 `hashed-password`（argon2/bcrypt），
  Caddy 已是 bcrypt，KasmVNC 已是 hash。磁盘上不再有登录明文。
- **VAULT_KEY** 独立，**绝不等于登录口令**；来源见 Track B；解锁时经 stdin/fd 喂给 gocryptfs，用后 zeroize。
- 收益：攻陷登录面（config.yaml、.kasmpasswd）不再等于拿到 vault 密钥；#3/#5 不再是密钥泄露点。
- 代价：两个 secret；无人值守重启的 UX 要重想（VAULT_KEY 从哪来，见 Track B）；必须拆掉
  `master_password_from_config()` 这个耦合（KasmVNC 的口令改由登录口令直接提供，与 vault 密钥脱钩）。

### Track B — 给 vault 密钥一个硬件/托管信任根（需 TPM2 或 KMS，**本地不可 PoC**）

- **B1 TPM2 封存**：用 TPM2 把 VAULT_KEY 封进一个 blob（`tpm2_create`/`tpm2_load` + PCR policy，
  或直接 `systemd-creds --with-key=tpm2`）。启动时只有 PCR 状态匹配（measured boot）才能解封，
  密钥静止时从不明文落盘、从不进环境，解封后经管道喂给 gocryptfs。
  注意：PCR 脆弱（内核/固件升级要重新封存）、TPM 的 DA lockout、容器要透传 `/dev/tpmrm0`
  （宿主需有 TPM；CI 里用 swtpm 模拟）。
- **B2 远程 KMS / Vault**：解锁时经隧道从 HashiCorp Vault / 云 KMS 取 VAULT_KEY，绑定一个身份
  token，绝不持久化。解锁需要网络，取不到就 fail-closed。适合部署计划里那些**没有 TPM 的 VPS**
  （Oracle Cloud ARM、Hetzner，见 TODO 的「多平台部署」）。
- 选型规则：有可用 TPM2 就用 B1；VPS/云上用 B2。（这台 Mac Mini 的 Secure Enclave 不以 Linux
  容器可见的 TPM2 形式存在，所以就算在真机上，Mac 宿主也更接近 B2 而非 B1。）

### Track C — 非 root + microVM（需嵌套虚拟化，**colima 上不可 PoC**）

- 容器以**非 root uid** 跑，顺带打掉「所有进程都是 root」这个假设——它也是 Phase 1 出口强制里
  按 uid 做防火墙规则的前提之一。（2026-11 更新：用户代码已经以 `dev` 运行，出口防火墙对它生效；
  容器里的 root 进程仍存在，且仍有 `CAP_SYS_ADMIN`。）
- 把容器裹进 **microVM（Firecracker / Kata）或 gVisor**，使「容器逃逸」或「读取解锁态内存密钥」
  要跨越一道 VM 边界，而不只是 namespace。这才是 #6 那个「解锁期间密钥在内存」残留风险的真正解法；
  Phase 3 的 capability 最小化只能削弱、不能消除它（SYS_ADMIN 为了 FUSE 必须留）。

## 每个泄露点由哪条线消掉

| 泄露点 | A 拆分 | B 信任根 | C microVM |
|--------|:------:|:--------:|:---------:|
| #1 `.env` 明文 | — | ✓（VAULT_KEY 不再进 `.env`） | — |
| #2 容器环境明文 | — | ✓（改为封存 blob / KMS 取用） | — |
| #3 config.yaml 明文 | ✓（已由网关做掉） | — | — |
| #4 日志（生成口令） | ✓（登录口令与密钥脱钩后，密钥从不打印） | ✓ | — |
| #5 `.kasmpasswd` | ✓（已由网关做掉） | — | — |
| #6 内存中的解锁态密钥 | — | 部分（解封即用即弃、zeroize） | ✓（VM 边界） |
| #7 sync-passphrase | 部分（已移出 vault、仅 root 可读；仍是一个落盘的独立文件，彻底的做法是同步密钥由 VAULT_KEY 派生，不落独立文件） | — | — |

## 不被本设计覆盖的东西（照旧）

- restic 备份仍是明文工作区，只靠 `RESTIC_PASSWORD`——与 vault 密钥是两套信任根，本设计不碰它
  （见 threat-model 的 Non-Guarantees）。
- `VAULT_GIT_REPO` 的 token、Clash 订阅 URL 仍是独立 credential，各有各的位置，不在此范围。

## 验证计划（真实硬件 / CI 才跑）

`src/tests/smoke/manual/tpm-seal-verify.sh`：在有 TPM2（或 swtpm）的宿主上，封存一个测试密钥并证明

1. PCR 匹配时解封成功；
2. PCR 变化后解封**失败**（measured-boot 绑定真的生效）；
3. 明文密钥全程不落盘（用 `strace` 审计 `open()`/`openat()` 写路径）；
4. 用 fd 喂进去的密钥能让 gocryptfs 正常挂载。

没有 TPM 的机器上脚本直接跳过并说明原因（这台开发机就是如此）。Track C 没有脚本：它要的是
部署形态（Firecracker/Kata 宿主或支持嵌套虚拟化的云实例），属于部署决策，不是单测能覆盖的。
