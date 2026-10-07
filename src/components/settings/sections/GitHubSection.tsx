import { useState } from 'react';
import { Github, Link, RefreshCw, User, KeyRound } from 'lucide-react';
import { FormGroup, Input, InputWithToggle, Button } from '../primitives';
import { useUiPrefsStore } from '@/lib/stores';
import { useTranslation } from '@/i18n';

type Status = { type: 'success' | 'warning' | 'error'; message: string } | null;

export function GitHubSection() {
    const { t } = useTranslation();
    const username = useUiPrefsStore((s) => s.githubUsername);
    const token = useUiPrefsStore((s) => s.githubToken);
    const repo = useUiPrefsStore((s) => s.githubRepoUrl);
    const showToken = useUiPrefsStore((s) => s.showGithubToken);
    const set = useUiPrefsStore((s) => s.set);

    const [testing, setTesting] = useState(false);
    const [status, setStatus] = useState<Status>(null);

    // Verifies the token against the GitHub API, then optionally checks repo access.
    const testConnection = async () => {
        setTesting(true);
        setStatus(null);
        try {
            const userRes = await fetch('https://api.github.com/user', {
                headers: { Authorization: `token ${token}`, Accept: 'application/vnd.github+json' },
            });
            if (!userRes.ok) {
                setStatus({ type: 'error', message: t('settings.github.connectionFailedStatus', { status: userRes.status }) });
                return;
            }
            const userData = await userRes.json();
            if (username && userData.login && userData.login.toLowerCase() !== username.toLowerCase()) {
                setStatus({ type: 'warning', message: t('settings.github.tokenMismatch', { tokenUser: userData.login, username }) });
                return;
            }

            const match = repo.match(/github\.com\/([^/]+)\/([^/]+?)(?:\.git)?\/?$/i);
            if (match) {
                const repoRes = await fetch(`https://api.github.com/repos/${match[1]}/${match[2]}`, {
                    headers: { Authorization: `token ${token}`, Accept: 'application/vnd.github+json' },
                });
                if (!repoRes.ok) {
                    setStatus({ type: 'warning', message: t('settings.github.repoAccessDenied', { login: userData.login, status: repoRes.status }) });
                    return;
                }
            }
            setStatus({ type: 'success', message: t('settings.github.connectedSuccess', { login: userData.login }) });
        } catch (e) {
            setStatus({ type: 'error', message: `Connection failed: ${e instanceof Error ? e.message : String(e)}` });
        } finally {
            setTesting(false);
        }
    };

    return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: '20px' }}>
            <FormGroup label={t('settings.github.username')} icon={<User size={15} />}>
                <Input value={username} onChange={(e) => set('githubUsername', e.target.value)} placeholder={t('settings.github.usernamePlaceholder')} />
            </FormGroup>

            <FormGroup label={t('settings.github.token')} icon={<KeyRound size={15} />}>
                <InputWithToggle
                    type={showToken ? 'text' : 'password'}
                    value={token}
                    onChange={(e) => set('githubToken', e.target.value)}
                    placeholder={t('settings.github.tokenPlaceholder')}
                    showValue={showToken}
                    onToggle={() => set('showGithubToken', !showToken)}
                />
            </FormGroup>

            <FormGroup label={t('settings.github.repoUrl')} icon={<Link size={15} />}>
                <Input value={repo} onChange={(e) => set('githubRepoUrl', e.target.value)} placeholder={t('settings.github.repoUrlPlaceholder')} icon={<Link size={16} />} />
            </FormGroup>

            {status && (
                <div style={{
                    padding: '12px',
                    background: status.type === 'success' ? 'color-mix(in oklab, var(--color-success) 12%, transparent)' : status.type === 'warning' ? 'color-mix(in oklab, var(--color-warning) 12%, transparent)' : 'color-mix(in oklab, var(--color-danger) 12%, transparent)',
                    border: `1px solid ${status.type === 'success' ? 'color-mix(in oklab, var(--color-success) 30%, transparent)' : status.type === 'warning' ? 'color-mix(in oklab, var(--color-warning) 30%, transparent)' : 'color-mix(in oklab, var(--color-danger) 30%, transparent)'}`,
                    borderRadius: 'var(--radius-sm)', fontSize: '13px',
                    color: status.type === 'success' ? 'var(--color-success)' : status.type === 'warning' ? 'var(--color-warning)' : 'var(--color-danger)',
                }}>
                    {status.message}
                </div>
            )}

            <Button
                icon={testing ? <RefreshCw size={16} style={{ animation: 'spin 1s linear infinite' }} /> : <Github size={16} />}
                fullWidth variant="secondary"
                onClick={testConnection}
                disabled={testing || !username || !token}
            >
                {testing ? t('settings.github.testing') : t('settings.github.testConnection')}
            </Button>
        </div>
    );
}
