# 《半条命 2》类型项目的引擎能力审计

## 结论

当前引擎能够制作一个小规模、可运行的 3D 第一人称叙事射击
vertical slice：关卡、角色控制、碰撞、刚体、脚本、动画运行时、音频、
导航、UI、流式加载、编辑器和 Windows 打包等主干已经存在。

当前引擎还不能支撑一个在内容规模、交互密度、角色表现、视觉效果和制作
工具链上接近 Valve《半条命 2》的完整商业项目。限制不只是“少几个游戏
玩法类”，而是下面这些生产级基础系统仍不完整。Valve 的代码、美术、关卡
和商标资产也不属于本仓库；这里的目标只能是制作同类型的原创游戏。

## 本轮已经补齐的基础层

| 能力 | 原状态 | 当前状态 |
|---|---|---|
| glTF 蒙皮顶点通路 | 导入器遇到 skin 会拒绝整个模型 | 保留 `JOINTS_0`、归一化 `WEIGHTS_0` 和节点 skin 索引；烘焙后自动选择 `Skinned64` GPU 顶点布局 |
| glTF 角色多产物导入 | 骨架和 clip 只能手工制作独立二进制资产 | 直接提取父子骨架、inverse-bind 和 LINEAR/STEP/CUBICSPLINE S/R/T 通道（STEP 保持键、CUBICSPLINE 确定性 60 Hz 烘焙并归一化四元数）；一次事务生成多 primitive mesh、每个 skin 的 skeleton/animation，并复制受约束的外部 buffer/image |
| 运行时存档快照 | 只有场景加载/失败回滚，没有玩家存档格式 | 新增版本化、限长、SHA-256 校验的 `SaveGameSnapshot`；保存 live ECS、世界原点、游戏状态和刚体瞬态 |
| 安全文件替换 | 无 | 同目录临时文件写入并 flush，旧文件临时备份，最终 rename 失败时回滚 |
| 大世界存档恢复 | 场景只能以零原点重新加载 | 存档同时恢复 `f64` 世界原点和 shift 计数，不会二次平移相对坐标 |
| 动态道具状态 | 场景只能保存组件初值 | 按 persistent ID 恢复刚体位置、旋转、线/角速度和睡眠状态 |
| C# 主动物理交互 | 只能做 ray/sphere/overlap 查询，不能推动物体 | 新增 `Physics.ApplyForce` / `ApplyImpulse` / `ApplyTorque` / `ApplyTorqueImpulse`，支持 `Entity` 或 persistent ID，带有限值、目标校验、每帧预算和安全物理步队列 |
| 持久化关节与抓取 | 后端关节只能用瞬态 handle 创建，不能保存、由脚本控制或实际断裂 | 新增 `PhysicsJoint` 场景组件、fixed/revolute/prismatic/spherical 增量同步、限制/马达、C# 创建/更新/移除/抓取 API、force/torque 断裂事件和存档重建 |
| 可破坏物与伤害通路 | 无统一生命值、伤害命令或碎片替换事务 | 新增 `Destructible` 场景组件、编辑器 Gameplay 创建入口、阈值/倍率/一次性破坏、C# `Damage.Apply` 与下一帧事件、破坏 prefab 原子替换、刚体速度/冲量继承、失败保留原物体及存档恢复 |
| 布娃娃基础 | 动画和刚体各自存在，无法交接骨骼权属 | 新增 `RagdollComponent` 刚体/约束定义、确定性图生成、动画→动态物理切换、物理 pose 回写蒙皮、C# 激活/恢复 API、定时回归混合和完整存档恢复 |
| 材质表面状态 | cooked material 只接受 opaque 且拒绝 double-sided | `MaterialSource-v0` 支持 Opaque/Masked/Blend、alpha cutoff 与双面；Vulkan/DX12 管线变体、alpha-test、透明排序/无深度写入、编辑器读写和 0.1 cooked 兼容已接通 |
| 自发光材质参数 | 无端到端通路 | 新增 RGB emissive factor、0.4 cooked schema 与 0.1/0.2/0.3 兼容；编辑器保存、Vulkan 48-byte UBO、DX12 208-byte 根常量和双后端 shader 已接通 |
| 完整基础 PBR 贴图槽 | 只有 base-color 纹理 | 新增法线、金属-粗糙度、AO 与自发光贴图；资源依赖/流式加载/编辑器、静态与蒙皮 Vulkan/DX12 描述符及 shader 全部接通，并保留旧 cooked 材质兼容 |
| 粒子与贴花基础 | 无完整运行时系统 | 新增可序列化 `ParticleEmitter` / `Decal` 组件、确定性 CPU 发射/突发/生命周期、锥形速度/加速度/尺寸插值、相机朝向四边形、视锥剔除、有限寿命贴花、内置透明材质与四边形，以及世界原点漂移一致性 |
| C# 角色控制命令 | 脚本只能直接改 Transform，绕过碰撞与状态机 | Script API 0.9 新增 `Character.Move` / `Jump` / `Control`；按持久 ID 解析 `CharacterController`，校验水平单位方向和 100 m/s 速度上限，下一模拟帧由控制器消费，主玩家镜像不会覆盖命令 |
| 项目交互约定 | 查询、关节和抓取 API 各自存在，缺少统一可用目标 | 新增可由编辑器 Gameplay 分类添加的 `engine.interactable`（提示/动作键/最大距离/可抓取）、命中元数据和 `Interaction.Probe/TryGetTarget/Grab/ReleaseGrab`；默认排除自身并沿用有界异步查询和持久关节 |

实现入口：

- `crates/engine-asset/src/gltf.rs`
- `crates/engine-animation/src/gltf_import.rs`
- `crates/engine-core/src/cooked_assets.rs`
- `crates/engine-core/src/savegame.rs`
- `crates/engine-core/src/game_loop.rs`
- `crates/engine-physics/src/world.rs`
- `crates/engine-physics/src/joints.rs`
- `crates/engine-physics/src/destruction.rs`
- `crates/engine-animation/src/ragdoll.rs`
- `crates/engine-core/src/ragdoll_runtime.rs`
- `crates/engine-vfx/src/lib.rs`
- `crates/engine-editor/src/material_editor.rs`
- `crates/render-vulkan/src/scene_renderer.rs`
- `crates/render-dx12/src/scene_renderer.rs`
- `crates/engine-scene/src/world/mod.rs`
- `crates/engine-script/src/gameplay.rs`
- `crates/sandbox/src/project_scripts.rs`
- `crates/sandbox/src/project_cli.rs`

## 仍缺失的生产级基础功能

### P0：做出可信 vertical slice 前应补齐

| 系统 | 已有基础 | 仍缺内容 |
|---|---|---|
| 角色资产导入 | glTF 多 mesh/skin 与 LINEAR/STEP/CUBICSPLINE animation 直接导入、inverse bind、蒙皮 GPU 布局、骨架/clip 运行时、状态机、IK、root motion | 重定向、压缩、事件轨与批量校验；morph target/表情 |
| 物理交互 | Rapier 刚体/碰撞体/查询、持久化关节、限制/马达、C# 施力/扭矩/抓取、实际断裂事件、`Destructible` 伤害累积与 prefab 碎片替换、速度/冲量继承、存档重建 | 自动碰撞冲量转伤害、材质抗性、运行时几何切割/预碎、破坏预览与关节网络压力可视化 |
| 布娃娃 | 显式骨骼 body/constraint 定义、自动生成持久刚体/关节图、动画/物理权属切换、物理 pose 回写、定时回归动画混合、C# 与存档支持 | 自动胶囊拟合、相邻 body 碰撞排除、joint-drive physical animation、编辑器 gizmo/限制预览、起身动画的 root/朝向对齐 |
| 粒子与贴花 | 可序列化 CPU 发射器、连续/突发发射、生命周期、速度锥/加速度/尺寸曲线、相机朝向与视锥剔除；mesh 贴花、法线偏移和定时回收；透明材质沿双后端排序路径绘制 | GPU 模拟、每粒子颜色/alpha 曲线、碰撞/子发射器、软粒子；体积投射贴花、接收层/法线融合、自动命中生成和专用编辑器 |
| 材质与特殊表面 | Vulkan/DX12 PBR forward、Opaque/Masked/Blend、alpha-test、双面管线、透明排序；base-color/法线/金属-粗糙度/AO/自发光贴图与自发光 RGB factor | masked shadow cutout、玻璃、水面、折射、clear coat、材质实例与导入切线 |
| 可玩角色交互 | character controller、输入、C# 生命周期、主动查询/冲量/关节/伤害命令、受限角色命令，以及 `Interactable` + 异步 use/grab 约定 | 第一人称视角/武器 viewmodel；具体武器数值和规则应留在项目 C# |

### P1：完整章节制作前应补齐

| 系统 | 已有基础 | 仍缺内容 |
|---|---|---|
| AI | navmesh、NavAgent、Idle/Patrol/Chase 基础行为 | 视听感知、遮挡、威胁记忆、掩体、战术槽位、小队协调、动态障碍、off-mesh link、门和电梯协同 |
| 过场与叙事 | 音频、动画、脚本、UI 可分别驱动 | 统一 timeline、镜头轨、角色/音频/事件轨、可跳过与存档恢复；对话、字幕、本地化和语音同步 |
| 光照与大关卡性能 | 方向光 3 级 CSM、世界 cell streaming、origin shift | 点光/聚光阴影、静态光照烘焙、反射探针、室内可见性/portal/PVS 或遮挡剔除、自动 LOD/HLOD |
| 载具 | 通用刚体与输入映射 | 轮胎/悬挂/传动、座位与上下车、相机、网络无关的可重复车辆控制器 |
| 存档产品层 | 本轮新增原生快照和文件格式 | C# 自定义状态回调、存档迁移策略、slot/quick-save/autosave、截图和 UI；streaming driver 请求状态目前重建而非逐字节恢复 |
| 制作工具 | 编辑器、prefab、材质面板、烘焙与打包 | 粒子/贴花/timeline/AI 调试工具、关节与 ragdoll authoring、关卡性能预算和可见性烘焙面板 |

### P2：接近成熟商业表现时需要

- 面部骨骼、morph 表情、视线和 lip-sync。
- 更完整的后处理：TAA、运动模糊、景深、体积雾、色彩分级工作流。
- 材质/着色器变体管理和平台级 shader cache。
- 更细的 CPU/GPU/内存/流式资源分析，以及长章节 soak 和故障注入。
- 无障碍设置、完整手柄体验、本地化字形和字幕布局。

## 哪些不应硬编码进引擎

武器、伤害、生命值、护甲、弹药、库存、任务、敌人具体决策树、谜题、
章节流程和对话条件属于游戏项目。引擎应提供稳定的实体、物理、动画、
音频、UI、存档和工具接口，由 C# 与项目数据组合这些规则。这样才不会把
一个通用引擎变成只能复制某一款游戏的专用代码库。

## 当前可达成的实际目标

以现在的代码，合理的下一里程碑是一个原创的 10–20 分钟单关卡：

1. 第一人称移动、交互射线和可推动刚体。
2. 一种基于冲量的“抓取/投掷”原型。
3. 一个通过单次 glTF/GLB 导入生成 mesh、骨架和 clip 的蒙皮敌人或 NPC。
4. 可累计伤害并替换为继承速度/冲量碎片 prefab 的破坏道具。
5. 可由命中冲量激活、驱动蒙皮姿态并混合恢复的 NPC 布娃娃。
6. 触发器、对话 UI、音频、导航追逐和场景切换。
7. quick-save/quick-load，通过项目自定义状态保存目标和库存。

要从该里程碑走向完整《半条命 2》量级项目，应按 P0 → P1 推进，而不是先
堆武器和关卡内容；否则角色、交互物理、视觉反馈和存档都会在内容扩张后
产生高成本返工。
