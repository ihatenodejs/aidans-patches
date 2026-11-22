# Play probe fixture

`aurora-profile.json` is the fixed Android device property set the native Play
probe sends to the Aurora token dispenser. It is a pinned Pixel 4a profile
(`sunfish`, Android 13, GSF/Vending versions included). The dispenser rejects
abbreviated profiles with HTTP 500, so these factual build properties are
compiled into `play.rs` with `include_str!`.

No account identity, token, email, or private key belongs in this directory.
