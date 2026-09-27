# CLI Reference

```
emberly                     Start (or offer to resume) an interactive session
emberly --plain             Run in plain line mode (no full-screen TUI)
emberly resume [id]         Resume the latest session, or one by id
emberly sessions            List saved sessions in this project
emberly export <path>       Export a session (latest, or --session <id>) to HTML
emberly init                Scaffold .agents/ (config, prompts, permissions)
emberly config show         Show the resolved configuration and its sources
emberly trust list          List trusted folders
emberly trust revoke <path> Revoke trust for a folder
emberly clean [<id>]        Reclaim scratch-space disk usage (all, or one session)
emberly --version           Print the version
emberly --help, -h          Show this help

  --provider <name>         Select the active provider profile for this run
  --model <name>            Override the model for this run
```

Plain mode is selected automatically when output isn't a terminal or
`NO_COLOR`/`TERM=dumb` are set — so Emberly degrades gracefully over pipes and
minimal terminals. Any subcommand with its own options — `config`, `trust`,
`export`, `clean` — also takes `--help`/`-h` for more on just that one.
