# 部署自动化脚本 - 设计逻辑

## Flow chart

![diagram](./imgs/flowchart.png)

```mermaid
flowchart TD

    A[Start Script] --> B[main function]
    B --> C[Ensure $DIR_BASE, $DIR_REPOS, $DIR_COPIES]
    C --> D{Loop forever or once}
    
    D --> E[Acquire $CI_LOCK - retry up to 100s]
    E -->|Failed| EX[Error and exit]
    E -->|OK| F[cd $DIR_REPOS]

    F --> G[Build repo list: REPO_WHITELIST or all git dirs]
    G --> H[For each repo in list]
    H -->|Repo has .git| I[fetch_and_check]

    I --> I1[Remove stale .git/index.lock]
    I1 --> J[git fetch -q --all --tags --prune --prune-tags]
    J --> K[For each remote branch]
    K -->|In BR_WHITELIST / BR_WHITELIST_PER_REPO or copy exists| L[checkout_and_copy_br]
    L --> L1[git archive ref \| tar -x to copy dir]
    L --> L2[Restart docker if configured]
    L --> M[Touch .living file]
    K --> M

    J --> N[get_release_tags: pattern, exclude, top-N, version-sorted]
    N --> O[For each release tag]
    O --> O1[checkout_and_copy_tag: create dir if missing, git archive tag \| tar -x]
    O1 --> O2{Is latest release?}
    O2 -->|Yes| O3[Update .prod.latest symlink]
    O3 --> O4[Restart docker if configured]
    O4 --> P[Touch .living file]
    O2 -->|No| O5[Skip docker for this tag]
    O5 --> P

    J --> Q[Clean up deprecated dirs in copies]
    Q --> Q1[.stopping? → clear dir, touch .skipping and .living]
    Q --> Q2[No .living? → rename to .to-be-removed]

    P --> R[Return to $DIR_REPOS]
    M --> R
    R --> S[Next repo]

    S --> H
    S --> T[Release $CI_LOCK]
    T --> U{SLEEP_TIME set and > 0?}
    U -->|No| V[Exit]
    U -->|Yes| W[sleep $SLEEP_TIME] --> D
```

### Source

Logic is implemented in `src/check-push.sh`. Config: `RELEASE_TAG_PATTERN`, `RELEASE_TAG_EXCLUDE_PATTERN`, `RELEASE_TAG_TOPN`, `REPO_WHITELIST`, `BR_WHITELIST`, `BR_WHITELIST_PER_REPO`, `DIR_BASE` → `DIR_REPOS`/`DIR_COPIES`, `CI_LOCK`.

Docker restart supports optional hook job files around restart:

- `*.docker.pre` (runs before `docker restart`)
- `*.docker.post` (runs after successful `docker restart`)

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
    participant Docker as _handle_docker()

    Main->>RepoLoop: Iterate repos (REPO_WHITELIST or all git dirs in $DIR_REPOS)
    RepoLoop->>Fetch: fetch_and_check(repo)

    Fetch->>Fetch: Remove .git/index.lock
    Fetch->>Fetch: git fetch -q --all --tags --prune --prune-tags

    loop For each branch (whitelist or existing copy)
        Fetch->>Branch: checkout_and_copy_br(repo, branch)
        alt First copy (whitelisted)
            Branch->>Branch: mkdir copy dir (no .skipping)
            Branch->>Branch: git archive ref | tar -x to copy dir
        else First copy (non-whitelisted)
            Branch->>Branch: mkdir copy dir + touch .skipping
        else Valid branch (no .skipping, no .debugging)
            Branch->>Branch: git archive origin/branch | tar -x (staging then mv, or overwrite if .no-cleanup)
            Branch->>Docker: Restart docker (if configured)
            Docker-->>Branch: done
        end
        Branch-->>Fetch: touch .living
    end

    loop For each release tag (pattern, exclude, top-N, version-sorted)
        Fetch->>Tag: checkout_and_copy_tag(repo, tag)
        Tag->>Tag: mkdir copy dir (if missing)
        Tag->>Tag: git archive tag | tar -x to copy dir
        alt Is latest release
            Tag->>Tag: Update .prod.latest symlink
            Tag->>Docker: Restart docker (if configured)
            Docker-->>Tag: done
        end
        Tag-->>Fetch: touch .living
    end

    Fetch->>Fetch: Cleanup old dirs: .stopping or no .living then rename to .to-be-removed

    Fetch-->>RepoLoop: return
    RepoLoop-->>Main: Done with repo

    Main->>Main: Remove $CI_LOCK
    Main->>Main: sleep $SLEEP_TIME (if set) → loop again
    Main->>Main: exit (if no sleep)