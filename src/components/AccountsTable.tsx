import type { AccountRow } from '../tauri-commands.ts';
import { formatTotals } from '../amount-utils.ts';

export function AccountsTable({
    accounts,
    onSelectAccount,
}: {
    accounts: AccountRow[];
    onSelectAccount: (name: string) => void;
}) {
    return (
        <table className="ledger-table">
            <thead>
                <tr>
                    <th>Account</th>
                    <th>Balance</th>
                    <th>Extraction</th>
                </tr>
            </thead>
            <tbody>
                {accounts.length === 0 ? (
                    <tr>
                        <td colSpan={3} className="table-empty">
                            No accounts found.
                        </td>
                    </tr>
                ) : (
                    accounts.map((account) => (
                        <tr key={account.name}>
                            <td>
                                <button
                                    className="link-button mono"
                                    onClick={() => {
                                        onSelectAccount(account.name);
                                    }}
                                >
                                    {account.name}
                                </button>
                            </td>
                            <td className="amount">
                                {formatTotals(account.totals)}
                            </td>
                            <td>
                                {account.unpostedCount > 0 ? (
                                    <span className="chip secret-chip warning">
                                        {account.unpostedCount} unposted
                                    </span>
                                ) : (
                                    <span className="status-dim">
                                        up to date
                                    </span>
                                )}
                            </td>
                        </tr>
                    ))
                )}
            </tbody>
        </table>
    );
}
