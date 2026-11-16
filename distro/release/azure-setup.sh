#!/usr/bin/env bash
# azure-setup.sh: what SlateOS's binary cache needs on Azure, once. Run it yourself,
# logged in with `az login` and `gh auth login`; it changes your storage account and
# your repository's secrets, and says each step before it does it.
#
#   distro/release/azure-setup.sh [ACCOUNT [RESOURCE-GROUP]]
#
# 1. The storage account may have public containers (it could not until now).
# 2. A container "slateos" whose blobs anyone can read (not list): the cache is
#    downloaded by every installed system, without an account.
# 3. An access policy "ci-upload" on that container: create and write blobs, nothing
#    else, until the date below. The Release workflow uploads with a signature made
#    from it; removing the policy takes that right away at once.
# 4. Two repository secrets: AZURE_STORAGE_SAS (that signature) and SLATEOS_CACHE_KEY
#    (the key the cache's paths are signed with, from ~/.config/slateos/cache.key).
# 5. A budget on the resource group: an e-mail to you when a month's cost passes
#    80 % and 100 % of BUDGET US dollars. It tells; it does not stop anything.
set -euo pipefail
account=${1:-sttest640a29}
group=${2:-test}
container=slateos
expiry=2027-11-30T00:00Z
budget=${BUDGET:-5}
key_file=${SLATEOS_CACHE_KEY_FILE:-$HOME/.config/slateos/cache.key}
repo=$(git -C "$(dirname "$0")" remote get-url origin | sed -E 's#.*github.com[:/]##; s#\.git$##')

step() { printf '\n== %s\n' "$*"; }
[ -r "$key_file" ] || { echo "$key_file is missing (the cache's secret key)" >&2; exit 1; }

step "1. $account: allow containers that anyone can read"
az storage account update -n "$account" -g "$group" --allow-blob-public-access true \
  --query "{account:name, publicContainersAllowed:allowBlobPublicAccess}" -o json

step "2. container $container, blobs readable by anyone"
az storage container create --name "$container" --account-name "$account" --public-access blob --auth-mode key -o none
az storage container show --name "$container" --account-name "$account" --auth-mode key \
  --query "{container:name, access:properties.publicAccess}" -o json

step "3. access policy ci-upload: create and write, until $expiry"
az storage container policy create --container-name "$container" --account-name "$account" \
  --name ci-upload --permissions cw --expiry "$expiry" --auth-mode key -o none ||
  az storage container policy update --container-name "$container" --account-name "$account" \
    --name ci-upload --permissions cw --expiry "$expiry" --auth-mode key -o none

step "4. repository secrets of $repo (the values are not shown)"
az storage container generate-sas --name "$container" --account-name "$account" --policy-name ci-upload \
  --https-only --auth-mode key -o tsv | gh secret set AZURE_STORAGE_SAS --repo "$repo"
gh secret set SLATEOS_CACHE_KEY --repo "$repo" < "$key_file"
gh secret list --repo "$repo"

step "5. a budget of $budget USD a month on resource group $group, with e-mail to you"
mail=$(az account show --query user.name -o tsv)
subscription=$(az account show --query id -o tsv)
body=$(python3 - "$budget" "$mail" "$(date -u +%Y-%m-01)" <<'PY'
import json, sys
amount, mail, start = float(sys.argv[1]), sys.argv[2], sys.argv[3]
def at(threshold):
    return {"enabled": True, "operator": "GreaterThan", "threshold": threshold, "thresholdType": "Actual", "contactEmails": [mail]}
print(json.dumps({"properties": {
    "category": "Cost", "amount": amount, "timeGrain": "Monthly",
    "timePeriod": {"startDate": start + "T00:00:00Z", "endDate": "2030-12-31T00:00:00Z"},
    "notifications": {"at80": at(80), "at100": at(100)}}}))
PY
)
az rest --method put -o none --body "$body" \
  --url "https://management.azure.com/subscriptions/$subscription/resourceGroups/$group/providers/Microsoft.Consumption/budgets/slateos-cache?api-version=2023-05-01" &&
  echo "budget set: an e-mail to $mail at 80 % and 100 % of $budget USD a month" ||
  echo "The budget could not be set from here (sponsored subscriptions do not always allow it): set one under Cost Management, Budgets, in the portal."

step "done"
echo "The cache's address: https://$account.blob.core.windows.net/$container/cache"
echo "It is what distro/release/cache.json names: $(grep -o '"url": "[^"]*"' "$(dirname "$0")/cache.json")"
