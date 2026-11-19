# Changelog

## 0.2.3

- Sync with [logdot/highfleet-qol `c07b521`](https://github.com/logdot/highfleet-qol/commit/c07b521): use shared native layouts for gun blocking and aircraft, and use the upstream skippable intro.
- Keep Allies-specific settings and behavior, including both sides' pre-fire line suppression, independent enemy sound suppression, manual editor elevation, `classic_kaz`, and the additional stock crash guards.

## 0.2.2

- Hide friendly and allied pre-fire gun warning lines as well as enemy lines
  when `enemy_laser_warning` is false. The sound option remains independent.

## 0.2.1

- Add an independent option to mute the enemy pre-fire sound warning on
  Steam 1.163.

## 0.2.0

- Integrate hf_intro 0.2.0: restore the native startup logos with keyboard or
  mouse skip on Steam 1.163.
- Add an option to hide enemy pre-fire gun warning lines on Steam 1.163.
