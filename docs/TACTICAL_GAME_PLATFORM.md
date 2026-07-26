# 回合制战术游戏基础平台

本模块面向 XCOM 一类“格子移动、掩体射击、战争迷雾、单位轮流行动”的游戏。它提供的是可组合的引擎能力，不包含某个具体项目的职业、武器数值、剧情或 UI 皮肤。

## 分层

| 层 | 位置 | 职责 |
| --- | --- | --- |
| 战术领域 | `engine-gameplay::tactics` | 棋盘、占位、预约、A*、可达区域、视野、回合、战斗、确定性随机、Utility AI |
| 通用运行时桥 | `engine-core` / `engine-script` | 指针、活动相机、世界射线、Logic 资产查询、完整世界存档 |
| C# 工具包 | `EngineTactics.cs` | 与领域层相同的项目侧组合工具，以及 JSON 快照 |
| 项目规则 | 游戏脚本和 Logic 资产 | 兵种、技能、任务目标、AI 权重、动画表现和界面 |

渲染、物理、资产与规则之间不互相持有具体实现。战术规则可以在无窗口、无 GPU 的测试中运行；表现层只消费规则事件。

## 使用的结构模式

- `TacticalBoard` 是空间状态的聚合根，统一维护格子、双向占位索引、预约和跨层连接。
- `TacticalAction` 与 `TurnDirector` 使用 Command 模式；动作先验证和排队，再由项目实现的 `TacticalActionExecutor` 解释。
- 移动成本、命中率、视线和 AI consideration 使用 Strategy 模式，可由项目替换。
- `TacticalSession` 是可序列化 façade，组合棋盘、视野、战斗单位和确定性随机状态。
- 脚本查询与存档采用延迟命令/结果事件，不允许脚本进程持有 ECS 或资产缓存指针。
- Logic 资产通过资产类型注册表加载，不在 cooked-asset 分支中硬编码专用缓存。

## Rust 入口

```rust
use engine_gameplay::tactics::*;

let mut board = TacticalBoard::default();
let start = GridCoord::new(0, 0, 0);
let goal = GridCoord::new(4, 2, 0);
board.insert_cell(TacticalCell::walkable(start, [0.0, 0.0, 0.0]));
// 添加其余格子、掩体边和楼梯/梯子连接……

let path = TacticalPathfinder::default().find_path(&board, start, goal, Some("unit-01"));
let reachable = TacticalPathfinder::default().reachable(&board, start, 12, Some("unit-01"));
```

坐标顺序、寻路平局、先攻平局和 Utility AI 平局都有稳定规则，配合 `DeterministicRng` 可用于回放、联机锁步验证和可复现测试。

## C# 入口

生成的 `EngineGameplay.dll` 包含两个受引擎管理的源文件：

- `EngineGameplay.cs`：进程桥、场景、输入、物理、UI、指针、相机、存档和资产查询。
- `EngineTactics.cs`：`Engine.Tactics` 命名空间下的战术工具包。

```csharp
using Engine;
using Engine.Tactics;

public sealed class TacticalController : EngineBehaviour
{
    private readonly TacticalSession _session = new(seed: 42);
    private LogicAssetQuery? _abilityQuery;

    public void OnCreate()
    {
        _abilityQuery = LogicAssets.Query("soldier-abilities");
    }

    public void OnUpdate(float dt)
    {
        if (Pointer.PrimaryPressed && Pointer.WorldRay is { } ray)
        {
            // 使用现有延迟 Physics.Raycast，把屏幕选择投向世界。
            Physics.Raycast(ray.Origin, ray.Direction, 1000f);
        }

        if (Input.WasPressed("quick_save"))
            Save.SaveJson("quicksave", _session.ToJson());

        if (Input.WasPressed("quick_load"))
            Save.Load("quicksave");

        if (Save.TryGetLoadedJson("quicksave", out var json))
        {
            var restored = TacticalSession.FromJson(json);
            // 用项目持有的 session 引用/状态容器接纳 restored。
        }
    }
}
```

`Save` 的操作在脚本回调结束后的帧边界执行。底层保存活动场景、世界原点、游戏状态、刚体状态和脚本 JSON，并使用大小限制、SHA-256 校验、临时文件替换与 `.bak` 回滚。结果在下一帧的 `Save.Events` 中返回。

窗口玩家默认把槽位写到项目的 `savegames` 目录；其他宿主必须显式调用 `GameLoop::set_script_save_directory`，从而由平台层决定用户数据目录。

## Logic 资产

`BehaviorTree`、`StateMachine`、`SkillGraph` 和 `QuestDialogue` 现在会被运行时加载到统一资产缓存。C# 使用 `LogicAssets.Query(id)`，下一帧通过 `TryGetJson`、`TryDeserialize<T>` 或 `Result` 取得结果。

Logic 资产只定义图结构、参数和条件。节点含义由项目的解释器或 `ITacticalActionExecutor` 决定，这样技能系统、AI 和剧情不会被固定成一种实现。

## 仍属于项目层的内容

以下内容不应进入通用引擎：具体的两行动点规则、职业树、武器表、命中公式覆盖、任务胜负条件、敌人 archetype、镜头手感、动画时间轴和战术 HUD。引擎已提供相应的策略接口、事件和数据通道，项目应在其上组合这些规则。
