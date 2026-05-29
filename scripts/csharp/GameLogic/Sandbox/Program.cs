using Engine;
using Sandbox;
using static System.Console;

// ── Demo: 模拟经营游戏脚本开发环境 ──────────────────────

// 模拟经营用的 Component（实际项目中定义在 GameLogic/Components.cs）
// 放在文件末尾的类型声明区

// 1. 创建沙盒世界
using var world = new SandboxWorld();

// 2. 创建模拟市民
var miner = world.SpawnWithId("miner-001",
    new Citizen { Name = "张三", Energy = 100, Mood = 80, Gold = 30 },
    new Position { X = 10, Y = 20 }
);

var builder = world.SpawnWithId("builder-001",
    new Citizen { Name = "李四", Energy = 90, Mood = 70, Gold = 20 },
    new Position { X = 5, Y = 5 }
);

// 3. 模拟游戏逻辑（C# 脚本开发者的日常工作）

WriteLine("\n── 模拟第 1 天 ──\n");

// 张三去采矿
MockEngineAPI.SetComponent(miner, new WorkCommand
{
    TaskType = "mine",
    Duration = 8f
});
WriteLine($"{GetName(miner)} 开始采矿");

// 李四去建造
MockEngineAPI.SetComponent(builder, new WorkCommand
{
    TaskType = "craft",
    Duration = 6f
});
WriteLine($"{GetName(builder)} 开始建造");

// 检查状态
world.Dump();

// 4. 模拟一帧过去
world.Tick();

// 5. 模拟玩家操作 ── 发工资
WriteLine("\n── 发工资 ──\n");

var citizen = MockEngineAPI.GetComponent<Citizen>(miner);
MockEngineAPI.SetComponent(miner, citizen with { Gold = citizen.Gold + 100 });

citizen = MockEngineAPI.GetComponent<Citizen>(builder);
MockEngineAPI.SetComponent(builder, citizen with { Gold = citizen.Gold + 80 });

// 6. 查看最终状态
world.Dump();

// 7. 验证结果
WriteLine("── 验证结果 ──\n");
var finalMiner = MockEngineAPI.GetComponent<Citizen>(miner);
var finalBuilder = MockEngineAPI.GetComponent<Citizen>(builder);

WriteLine($"张三 金币: {finalMiner.Gold}  (预期: 130) {(finalMiner.Gold == 130 ? "✅" : "❌")}");
WriteLine($"李四 金币: {finalBuilder.Gold} (预期: 100) {(finalBuilder.Gold == 100 ? "✅" : "❌")}");

WriteLine($"\n总计实体数: 2");
WriteLine("\n沙盒运行完成。脚本逻辑全部在纯 C# 环境中验证，未编译 Rust 引擎。");

// ── 辅助方法 ──

string GetName(string entityId)
    => MockEngineAPI.GetComponent<Citizen>(entityId)?.Name ?? entityId;

// ── 模型定义（实际项目中这些会定义在 GameLogic/Components.cs 里） ──

/// 员工状态组件
public record Citizen
{
    public string Name { get; init; } = "";
    public float Energy { get; set; } = 100f;
    public float Mood { get; set; } = 80f;
    public int Gold { get; set; } = 50;
}

/// 位置组件
public record Position
{
    public float X { get; set; }
    public float Y { get; set; }
}

/// 工作任务组件（脚本写指令，DOTS 消费）
public record WorkCommand
{
    public string TaskType { get; init; } = ""; // "mine", "craft", "rest"
    public float Duration { get; init; } = 5f;
}
