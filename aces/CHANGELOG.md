# 0.1.0

- Extract Shipworks custom rosters, Airstrike, Missile Defense, Silent Strike,
  and the optional crash button from Allies into `aces.dll`.
- Add `Modloader/config/aces.json` with `enable` and `crash_button`.
- Use the stock friendly reserve when Allies is absent. When Allies is loaded,
  keep automatic test wingman selection and restore the prior selection on exit.
