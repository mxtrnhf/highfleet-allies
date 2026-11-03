# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

## Changes since v0.14.10

- **Pre-combat buttons now follow the ship cards smoothly.** Dragging or shuffling the ship cards no longer leaves the ALLY and carrier plane-loadout buttons behind to snap into place after release; they move with the cards every frame.
- **Plane loadout columns stay with their carrier.** When a carrier swaps slots with another ship, its plane buttons and labels travel with the carrier instead of jumping to the card that took over its old slot. This also works for the flagship in slot 1, which has no ALLY toggle.
- **Clicks stay correct while a column moves.** Clicking a plane button during the brief transition still changes the loadout of the ship the button displays, not the card underneath it.
- No configuration changes; existing `ally.json` files work as-is.

## Pack contents

hf_ally @ALLY_VERSION@, hf_ai @AI_VERSION@ (unchanged since v0.14.10), and the bundled QoL build (unchanged).

## Installation

Back up your saves and configs. Extract the pack into the folder containing Highfleet.exe, keeping only one ally DLL and one AI DLL. The bundled ally/AI configs replace existing files if extracted over them; merge your custom values instead if needed.

The QoL build is optional. Keep its flare crash fix enabled. Its full JSON file is a reference, not an active config.
