# Quickstart: Keel workspaces, repository catalog and hosts

1. As an owner, open the workspace switcher in the top bar, choose **Create workspace…**, and
   name it `homelab`. It becomes current, with a `default` project.
2. Go to **Settings → Workspace**. Under Repository, enter the Ansible repository URL and
   branch, then create a `git_https_token` credential inline. Confirm. Keel syncs it.
3. Under Forge connection, choose Forgejo, create a `forge_token` credential (a token with
   repository write), and press **Test**.
4. Open **Repository**. Playbooks, Roles and Inventory list what is in the repository. Pick an
   undeclared playbook, choose **Draft job**, and copy or propose the draft.
5. Open **Repository → Secrets**. Each sops file shows its recipients and the hosts that can
   decrypt it.
6. Open **Hosts → Add host**. Name it `newbox`, pick group `compose_hosts` and profile
   `prox-vm`, and confirm. Run the enrolment command on the box, then merge the pull request.
   Once Keel syncs, the host turns from *pending inventory* to ready.
7. When the box reports its age key, use **Propose sops recipient** on the host. After that pull
   request merges, run `sops updatekeys` for the listed files from a machine that can decrypt.
