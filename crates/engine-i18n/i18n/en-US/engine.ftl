# [ENGINE] en-US message corpus — the default locale (spec II.6).
# This file is the fallback every other locale resolves against.

# --- Application chrome ---
app-title = Engine
menu-file = File
menu-edit = Edit
save-button = Save
quit-button = Quit

# --- Greetings and status ---
welcome = Welcome to the engine, { $name }!
project-opened = Opened project { $project }.

# --- Plural selection ---
entities-selected =
    { $count ->
        [0] No entities selected
        [one] { $count } entity selected
       *[other] { $count } entities selected
    }

assets-imported =
    { $count ->
        [one] Imported one asset
       *[other] Imported { $count } assets
    }

# --- Gender / literal selection ---
player-finished =
    { $gender ->
        [male] He finished the level
        [female] She finished the level
       *[other] They finished the level
    }
