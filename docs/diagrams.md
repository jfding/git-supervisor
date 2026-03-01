# 部署自动化脚本 - 设计逻辑

## Flow chart

![diagram](./imgs/flowchart.png)

```mermaid
flowchart TD

    A[Start Script] --> B[main function]
    B --> C[Ensure $DIR_REPOS exists]
    C --> D{Loop forever or once}
    
    D -->|Lock exists| D1[Sleep until lock released] --> D
    D -->|No lock| E[Create $CI_LOCK]

    E --> F[cd $DIR_REPOS]
    F --> G[For each repo in $DIR_REPOS]
    G -->|Repo has .git| H[fetch_and_check]
  

    H --> I[Remove stale .git/index.lock]
    I --> J[git fetch --all --tags]
    J --> K[For each remote branch]
    K -->|Whitelisted or copy exists| L[checkout_and_copy_br]
    L --> L1[Update copy dir & rsync files]
    L --> L2[Run post script if exists]
    L --> L3[Restart docker if configured]
    L --> M[Touch .living file]
    K --> M

    J --> N[For each release tag matching vX.Y.Z]
    N --> O[checkout_and_copy_tag]
    O --> O1[Create copy dir if missing]
    O --> O2[rsync files]
    O --> O3[Run post script if exists]
    O --> O4[Restart docker if configured]
    O --> P[Touch .living file]

    J --> Q[Clean up deprecated dirs]
    Q --> Q1[If dir has no .living → rename to .to-be-removed]

    P --> R[Return to $DIR_REPOS]
    M --> R
    R --> S[Next repo]

    S --> G
    S --> T[Remove $CI_LOCK]
    T --> U{SLEEP_TIME set?}
    U -->|No| V[Exit]
    U -->|Yes| W[sleep $SLEEP_TIME] --> D
```

### Source

## Sequence diagram

![diagram](./imgs/seqdiagram.png)

```mermaid
%% ================================
%% SEQUENCE DIAGRAM (function calls)
%% ================================
sequenceDiagram
    participant Main as main()
    participant RepoLoop as for each repo
    participant Fetch as fetch_and_check()
    participant Branch as checkout_and_copy_br()
    participant Tag as checkout_and_copy_tag()
    participant Post as _handle_post()
    participant Docker as _handle_docker()

    Main->>RepoLoop: Iterate repos in $DIR_REPOS
    RepoLoop->>Fetch: fetch_and_check(repo)

    Fetch->>Fetch: Remove .git/index.lock
    Fetch->>Fetch: git fetch --all --tags

    loop For each branch
        Fetch->>Branch: checkout_and_copy_br(repo, branch)
        alt First copy (whitelisted)
            Branch->>Branch: mkdir copy dir (no .skipping)
            Branch->>Branch: git archive → copy files
        else First copy (non-whitelisted)
            Branch->>Branch: mkdir copy dir + .skipping
        else Valid branch (no .skipping)
            Branch->>Branch: git archive / update files
            Branch->>Post: Run post script (if exists)
            Post-->>Branch: done
            Branch->>Docker: Restart docker (if configured)
            Docker-->>Branch: done
        end
        Branch-->>Fetch: touch .living
    end

    loop For each release tag
        Fetch->>Tag: checkout_and_copy_tag(repo, tag)
        Tag->>Tag: mkdir copy dir (if missing)
        Tag->>Tag: git checkout tag + rsync files
        Tag->>Post: Run post script (if exists)
        Post-->>Tag: done
        Tag->>Docker: Restart docker (if configured)
        Docker-->>Tag: done
        Tag-->>Fetch: touch .living
    end

    Fetch->>Fetch: Cleanup old dirs (no .living → rename)

    Fetch-->>RepoLoop: return
    RepoLoop-->>Main: Done with repo

    Main->>Main: Remove $CI_LOCK
    Main->>Main: sleep $SLEEP_TIME (if set) → loop again
    Main->>Main: exit (if no sleep)