# 大型 3D 游戏渲染架构

本文记录引擎面向大型 3D RPG、战术游戏和开放场景的渲染边界。通用渲染能力属于引擎；角色风格、技能表现、镜头语言和关卡灯光属于游戏项目。

## 数据流与职责

```text
Scene / Animation / VFX
          |
          v
RenderFrameInput（后端无关、严格校验）
          |
          v
Renderer + Render Graph
          |
          +--> Vulkan
          `--> DirectX 12
```

- `engine-scene` 负责相机、灯光、可见性、LOD/HLOD 和渲染设置的提取。
- `engine-animation` 负责蒙皮、骨骼调色板和 morph 权重。
- `engine-vfx` 负责粒子状态与批次，不持有 GPU 对象。
- `engine-renderer` 定义帧契约、资源上传、集群灯光、GPU 粒子参数、透明策略和 Render Graph。
- Vulkan 与 DX12 只负责资源生命周期、描述符、管线、命令录制和提交。

## 已实现的规模化能力

| 领域 | 当前能力 |
| --- | --- |
| 多光源 | Vulkan 与 DX12 共用集群划分和固定 GPU ABI；方向光、点光与聚光灯按屏幕分块和对数深度切片选择 |
| 透明 | 默认保留从远到近的 `Sorted` 模式；`WeightedBlendedOit` 使用颜色累积与光学深度 MRT，在 HDR 合成阶段解析，不依赖对象顺序 |
| 粒子 | `Cpu` 模式保留确定性实例流；`Gpu` 模式由 `InstanceID` 和 128 字节解析参数直接求值，后端不支持时自动执行确定性 CPU 回退 |
| HLOD | cooker 按空间、材质和渲染层自动聚类，合并变换后的网格，执行确定性顶点聚类简化，写出标准 cooked mesh，并幂等回写 source/proxy 场景组件 |
| HDR 与后处理 | HDR 前向目标、ACES/Reinhard/直出、Bloom、颜色校正和 Vignette |
| 环境光照 | GGX 预过滤 RGBA16F cubemap、天空盒、IBL 与局部反射探针选择 |
| 角色 | PBR、高级材质参数、蒙皮和每集合最多 8 个 morph target |
| 批处理 | 静态实例化、CPU/GPU 粒子批次、LOD/HLOD 选择、视锥与层剔除 |

## 加权混合 OIT

`SceneSettings.transparency_mode` 和 `RenderOptions.transparency_mode` 支持：

- `Sorted`：兼容原有 Alpha Blend 行为。
- `WeightedBlendedOit`：适合烟雾、魔法、植被和大量半透明粒子。

HDR 前向阶段固定声明三个颜色输出：

1. `hdr_color`
2. `oit_accumulation`
3. `oit_optical_depth`

普通管线只写第一个目标；OIT 管线关闭 HDR 写入，并以 `ONE + ONE` 独立混合写入后两个目标。ToneMap 在 Bloom 和色调映射前解析透明层。`DirectToSwapchain` 没有解析阶段，因此会拒绝 OIT 配置。

## GPU 粒子与回退

`ParticleSimulationMode::Gpu` 不维护逐粒子 CPU 数组。发射器只保存时间、发射范围、寿命、速度、颜色、湍流和确定性种子。Vulkan 和 DX12 顶点着色器根据实例序号计算存活、位置、旋转、尺寸和颜色。

`BackendRenderer::supports_gpu_particle_simulation` 用于能力协商。缺少该能力的后端会在前端展开同一解析模型，因此测试、无头后端和未来平台不会静默丢失粒子。

## 自动 HLOD 烘焙

资产侧入口：

- `bake_hlod_proxies`
- `bake_hlod_scene`
- `apply_hlod_bake_to_scene`
- `write_hlod_proxy_artifacts`

编辑器侧入口：

- `bake_scene_hlod_assets`

烘焙只合并静态 PBR 网格；不同材质或渲染层不会进入同一代理。所有 ID、聚类顺序和简化结果均为确定性输出。代理写入成功后才更新场景，避免场景引用半成品资产。

## 脚本工具

生成的 `EngineRendering.cs` 提供：

- `LodGroupSettings`、`ApplyLodGroup` 和 `QueryLodGroup`
- `HlodClusterSettings`、`ApplyHlodCluster` 和 `QueryHlodCluster`
- `ParticleEmitterSettings`、`ParticleSimulationMode`
- `ApplyParticleEmitter` 和 `QueryParticleEmitter`

脚本通过组件命令工作，不接触 GPU 句柄、描述符或后端对象。

## 尚未覆盖

当前仍未实现硬件光追、实时全局光照、体积云、电影级毛发、透明表面精确折射/色散、粒子碰撞与子发射器。这些属于后续画质或特效层，不再是制作大型 RPG 基础玩法的结构性阻塞项。
