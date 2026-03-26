# Changelog

## [0.5.0](https://github.com/OscarGTH/upcloud-migrator/compare/upcloud-migrate-v0.4.0...upcloud-migrate-v0.5.0) (2026-03-26)


### Features

* add better kubernetes pricing info and correct plan name. ([5bf6946](https://github.com/OscarGTH/upcloud-migrator/commit/5bf6946e6461b896efa667e6553279a11357457e))


### Bug Fixes

* comment kubernetes plan by default ([c1c167f](https://github.com/OscarGTH/upcloud-migrator/commit/c1c167f571e810d3056a570897cdba7f1c419f87))

## [0.4.0](https://github.com/OscarGTH/upcloud-migrator/compare/upcloud-migrate-v0.3.0...upcloud-migrate-v0.4.0) (2026-03-26)


### Features

* **style:** add better colors for dark and light terminals, and make the terminal color be auto detected. ([b5adedc](https://github.com/OscarGTH/upcloud-migrator/commit/b5adedc215f7831f5bd394be0922a92c7d2da639))

## [0.3.0](https://github.com/OscarGTH/upcloud-migrator/compare/upcloud-migrate-v0.2.0...upcloud-migrate-v0.3.0) (2026-03-26)


### Features

* only add lifecycle ignore changes to subnets when kube is included. ([4901723](https://github.com/OscarGTH/upcloud-migrator/commit/49017233659d4d5fa525ad20037d50f8dd5eb21e))

## [0.2.0](https://github.com/OscarGTH/upcloud-migrator/compare/upcloud-migrate-v0.1.0...upcloud-migrate-v0.2.0) (2026-03-26)


### Features

* add tf graph export to mermaid diagram. ([87672da](https://github.com/OscarGTH/upcloud-migrator/commit/87672dad90a333a3a2543cf226f036a2c7aee638))
* add the first MVP of the app (should have probs used git earlier already haha) ([222c68e](https://github.com/OscarGTH/upcloud-migrator/commit/222c68ebe0ff1a8183b65fc032f3abf29f5a1683))
* limit the scope a bit, improve subnet assignments, add demo tf project. ([2589706](https://github.com/OscarGTH/upcloud-migrator/commit/2589706bbeb4f0e39a5e670b3862648cd5e9dcbd))
* make variable detectin work better. add animation for generation. fix minor bugs. reduce note lengths. ([3802de6](https://github.com/OscarGTH/upcloud-migrator/commit/3802de6a36fae8cabd821d07123bb098a65284ac))


### Bug Fixes

* add pricing calculator and terraform formatting in the end. align plan names. rework AI chat. remove Validate with AI feature. ([eb18292](https://github.com/OscarGTH/upcloud-migrator/commit/eb182921632e37a48bfe7006a8faa50f229fedff))
* fix bug where storage resources did not get count attribute. add note to manual cert bundle that it has to be base64 filepath. ([cf9a946](https://github.com/OscarGTH/upcloud-migrator/commit/cf9a946e016b57e888cd3719e00d9b1f2df0b067))
* fix bugs to make networks be resolved to correct servers, adjust vm size equivalence table, fix bug crashing TUI on resizing it, improve README. ([95c1b9b](https://github.com/OscarGTH/upcloud-migrator/commit/95c1b9b5dd0b2be7a613e28ff3b07e07703d5f30))
* fix git config in automated release. ([d263520](https://github.com/OscarGTH/upcloud-migrator/commit/d263520f3d491902be80232b3a0b43d8e64dfd97))
* make demo use free IPs, fix bugs with permission in postgres parameters, add metadata true to all servers. add basic hello world to aws demo infra. ([bcab13b](https://github.com/OscarGTH/upcloud-migrator/commit/bcab13ba08cc136699ce092834072fdd7c68798f))
* make kuberenetes conversion more accurate. ([3ec0773](https://github.com/OscarGTH/upcloud-migrator/commit/3ec07738c40c2d069bf52058bcc5a663758d1cf3))
* make loadbalancer use port 80 ([489bbe6](https://github.com/OscarGTH/upcloud-migrator/commit/489bbe6cdfd07115da081afa4da68b07c91879d8))
* remove ai default url, add fix for including all source files (like json and shell scripts) ([1538ac7](https://github.com/OscarGTH/upcloud-migrator/commit/1538ac796c107b0d17ea359675642be89b82497b))
