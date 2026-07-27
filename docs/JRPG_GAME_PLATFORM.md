# 队伍制 JRPG 游戏基础平台

本平台面向《最终幻想》一类 3D 探索、队伍成长、指令/ATB 战斗、剧情对话和大型演出的角色扮演游戏。这里提供可复用的引擎规则、数据通道和脚本工具，不把某个项目的职业、数值、剧情或 UI 皮肤固化进引擎。

## 审查结论

原引擎已经拥有场景、Prefab、角色控制、导航、物理、骨骼动画、UI、音频、Logic 资产和完整世界存档，但缺少把这些底层系统组织成 JRPG 的规则层；C# 项目只能自行拼接字典和 JSON，也没有安全的动画专用入口。

本次补齐：

| 能力 | Rust 领域入口 | C# 工具 |
| --- | --- | --- |
| 角色、等级、经验、能力学习 | `jrpg::CharacterProgress` | `Engine.Jrpg.CharacterProgress` |
| 队伍、候补、货币 | `jrpg::Party` | `Party` |
| 物品、堆叠、装备、属性修正 | `jrpg::Inventory` | `Inventory` |
| 指令/ATB 战斗 | `jrpg::BattleSession` | `BattleSession` |
| 技能、MP、物理/魔法、元素、暴击 | `BattleEffect` / `BattleFormula` | `BattleEffect` / `IBattleFormula` |
| 状态叠层、持续时间、周期效果 | `StatusDefinition` | `StatusDefinition` |
| 遭遇进度与权重阵型 | `EncounterMeter` / `EncounterTable` | 同名工具 |
| 任务、目标、分支条件 | `StoryState` / `QuestDefinition` | `QuestJournal` / `StoryState` |
| 对话图与本地化 key | `DialogueRunner` | `DialogueRunner` |
| 本地化回退与 token 插值 | `LocalizationCatalog` | `LocalizationCatalog` |
| 可存档过场时间线 | `SequenceRunner` | `SequenceRunner` |
| 数据校验 | `JrpgDatabase` | `JrpgDatabase` |
| 完整项目规则快照 | `JrpgSession` | `JrpgSession` |
| 脚本音频、动画 | 原有组件与专用命令 | `EngineBehaviour.Audio` / `Animation` |

## 架构

- `Party`、`Inventory`、`BattleSession` 是各自状态边界的聚合根，负责维护双向关系、容量和阶段不变量。
- 战斗命令先验证再排队，随后统一解析并生成事件，使用 Command 模式。
- 经验曲线和战斗公式使用 Strategy 接口，项目可以替换而不修改会话状态结构。
- `JrpgDatabase` 是项目 Logic 资产的 typed façade，在进入运行时前检查定义 key 和跨表引用。
- 任务、对话和过场只生成 `NarrativeCommand` / `SequenceCommand`；项目表现层负责镜头、字幕、特效和 UI。
- `JrpgSession` 不持有 ECS、渲染器或音频句柄，因此能够直接进入现有 checkpoint JSON。

## C# 使用

Script API 版本为 `0.15.0`，生成的 `EngineGameplay.dll` 包含：

- `EngineGameplay.cs`：场景、输入、物理、UI、存档、资产查询、音频和动画桥。
- `EngineRules.cs`：确定性随机与权重表。
- `EngineTactics.cs`：战术游戏工具。
- `EngineJrpg.cs`：队伍制 RPG 工具。
- `EngineRendering.cs`：LOD/HLOD 与粒子发射器配置工具；粒子支持 CPU/GPU 模式、尺寸/颜色生命周期、阻力和确定性湍流。场景透明策略支持排序 Alpha 与加权混合 OIT。

```csharp
using Engine;
using Engine.Jrpg;

public sealed class GameDirector : EngineBehaviour
{
    private JrpgSession _session = new(seed: 42);
    private LogicAssetQuery? _databaseQuery;

    public void OnCreate()
    {
        _databaseQuery = JrpgScriptTools.QueryDatabase(this, "jrpg-database");
    }

    public void OnUpdate(float deltaTime)
    {
        if (_databaseQuery is { } query &&
            JrpgScriptTools.TryGetDatabase(this, query, out var database))
        {
            if (!_session.Party.Actors.ContainsKey("hero"))
                _session.Recruit(database, "hero");
            _databaseQuery = null;
        }

        if (Input.WasPressed("quick_save"))
            JrpgScriptTools.SaveSession(this, "quick", _session);

        if (Input.WasPressed("quick_load"))
            Save.Load("quick");

        if (JrpgScriptTools.TryRestoreSession(this, "quick", out var restored))
            _session = restored;
    }

    public void PlayAttack(Entity actor)
    {
        Animation.PlayClip(actor, "battle.attack", looping: false);
        Audio.Play(actor, "sfx.sword", volume: 0.8f);
    }
}
```

`Animation` 使用专用延迟命令修改 `AnimationPlayer`，不会通过通用 component map 覆盖状态机实例、姿态缓存或 ragdoll 所有权。`Audio` 是对已注册 `engine.audio_source` 的类型化 façade。

## 项目仍需制作的内容

以下属于游戏产品而不是通用引擎缺口：

- 具体职业、晶石/技能盘、召唤兽、武器表和最终伤害公式。
- 世界地图、关卡、NPC、敌人 AI、Boss 阶段和遭遇配置。
- 剧本、翻译文本、配音、镜头表、动作与面部动画资源。
- 战斗 HUD、菜单视觉、镜头手感、转场特效和可访问性设计。
- 资产规模下的流式预算、平台性能调优和内容生产流水线。

过场命令中的镜头混合、画面淡入淡出和自定义命令会返回项目表现层处理；通用 runner 只负责确定性的时间、顺序和存档恢复。
