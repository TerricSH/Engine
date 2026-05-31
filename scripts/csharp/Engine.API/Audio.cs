namespace Engine;

/// <summary>
/// C# wrapper for the engine's audio system.
/// Provides play/stop/volume control for game scripts.
/// </summary>
public class Audio
{
    private IntPtr _enginePtr;

    internal Audio(IntPtr enginePtr)
    {
        _enginePtr = enginePtr;
    }

    /// <summary>
    /// Play a sound clip by asset path.
    /// Returns a handle ID (0 on failure) that can be passed to
    /// <see cref="Stop"/> or <see cref="SetVolume"/>.
    /// </summary>
    public ulong PlaySound(string clipAsset, float volume = 1.0f, bool looping = false)
    {
        return EngineAPI.audio_play_sound(_enginePtr, clipAsset, volume, looping);
    }

    /// <summary>
    /// Stop a playing sound by handle ID.
    /// </summary>
    public void Stop(ulong handleId)
    {
        EngineAPI.audio_stop_sound(_enginePtr, handleId);
    }

    /// <summary>
    /// Set the volume of a playing sound by handle ID (0.0–1.0).
    /// </summary>
    public void SetVolume(ulong handleId, float volume)
    {
        EngineAPI.audio_set_volume(_enginePtr, handleId, volume);
    }

    /// <summary>
    /// Set the global master volume (0.0–1.0).
    /// </summary>
    public void SetMasterVolume(float volume)
    {
        EngineAPI.audio_set_master_volume(_enginePtr, volume);
    }
}
