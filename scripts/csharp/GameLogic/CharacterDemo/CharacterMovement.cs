using Engine;

namespace GameLogic;

/// <summary>
/// Sample character movement script demonstrating the C# character API.
/// </summary>
public class CharacterMovement
{
    private readonly CharacterController _controller;
    private int _frameCount;
    private int _previousState = -1;

    public CharacterMovement(CharacterController controller)
    {
        _controller = controller;
    }

    /// <summary>Called every frame to update character movement.</summary>
    public void Update(float dt, float inputX, float inputZ, bool wantsJump)
    {
        _frameCount++;

        // 1. Handle jump input
        if (wantsJump && _controller.IsGrounded)
        {
            bool jumped = _controller.Jump();
            if (jumped)
                Console.WriteLine($"[Frame {_frameCount}] Jump initiated");
        }

        // 2. Move the character
        bool moved = _controller.Move(inputX, inputZ, 5.0f, dt);
        if (moved && _frameCount % 60 == 0)
        {
            Console.WriteLine($"[Frame {_frameCount}] Moving: vel=({_controller.VelocityX:F2}, {_controller.VelocityY:F2}, {_controller.VelocityZ:F2})");
        }

        // 3. Log state transitions
        int currentState = (int)_controller.CurrentMoveState;
        if (currentState != _previousState)
        {
            string stateName = currentState switch
            {
                0 => "Grounded",
                1 => "Jumping",
                2 => "Falling",
                3 => "Landing",
                4 => "Free",
                _ => "Unknown"
            };
            Console.WriteLine($"[Frame {_frameCount}] State → {stateName}");
            _previousState = currentState;
        }
    }
}
