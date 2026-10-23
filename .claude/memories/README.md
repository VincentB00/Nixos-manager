# Knowledge transfer notes

Handoff notes for anyone (human or Claude session) picking up Nixos-manager.
Read in order. Each file is one topic. Update the relevant file whenever a
decision is made, something is learned, or work is completed. Dates are absolute.

| File | What it covers |
|---|---|
| 01-project-goal.md | What we are building, the hard requirements, and why |
| 02-vincent-nixos-setup.md | How Vincent's real NixOS config is structured today |
| 03-stack-decisions.md | Tech stack, alternatives considered, and reasons |
| 04-architecture.md | Daemon + web UI design, privilege model, repo layout |
| 05-status-and-next-steps.md | Where we are, open decisions, what to do next |
| 06-features.md | Startup flow, JSON state, Feature 1 (discover/map/edit), guided actions |
| 07-implementation-status.md | What is built, where it lives, gaps, how to run |
| 08-wizard.md | The guided "Add something" wizard: recipes, option index, package search, merge-in edits |
| 09-flake-and-channels.md | Flake page, channel management, switch-to-flakes template |
| 10-package-sets.md | pkgs / pkgs-<channel> selection in wizard and form, per-set package search |
| 11-organize.md | Organize button: group dotted settings into nested blocks |
| 12-secrets.md | Managed secrets: passphrase-encrypted secrets.age, deploy to /var/lib/nixos-manager/secrets |
| 13-validate.md | Validate: syntax check + evaluation/dry build on a staged copy, no sudo |
| 14-remote.md | Deploy targets (--target-host) and remote builders (buildMachines) |
| 17-tui.md | Terminal UI (`./run.sh --tui`): pick folder, build/deploy, validate, generations, job log |
| 16-git.md | Git page: branches, status/diffs, generated commit messages, push/fetch/pull, askpass credentials |
| 15-hardware.md | hardware-configuration.nix rules: system copy is the source of truth; sync on open, confirm on local build, target's own file for remote deploys |

Conventions for these notes:
- One topic per file, numbered for reading order.
- Record the *why* behind decisions, not just the what.
- When something changes, edit the file in place and note the date.
- Never store secrets here.
