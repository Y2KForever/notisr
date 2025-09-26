import { useEffect, useRef, useState } from 'react';
import './App.css';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { LogIn } from './views/LogIn';
import { List } from './views/List';
import { Menu } from './components/Menu';
import { Separator } from '@/components/ui/separator';
import { check } from '@tauri-apps/plugin-updater';
import { ask, message } from '@tauri-apps/plugin-dialog';
import { relaunch } from '@tauri-apps/plugin-process';

export const checkForUpdate = async (onUserClick: false) => {
  const update = await check();
  if (update) {
    const yes = await ask(`Update to ${update.version} is available! \n\n\ Release notes: ${update.body}`, {
      title: 'Update available',
      kind: 'info',
      okLabel: 'Update',
      cancelLabel: 'Cancel',
    });
    if (yes) {
      await update.downloadAndInstall();
      await relaunch();
    }
  } else if (onUserClick) {
    await message('You are on the latest version', {
      title: 'No Update Available',
      kind: 'info',
      okLabel: 'OK',
    });
  }
};

export const App = () => {
  const [layout, setLayout] = useState<string>('list');
  const [loading, setLoading] = useState<boolean>(true);
  const hasCheckedUpdate = useRef(false); // Guard strictMode
  const version = useRef<string | undefined>(undefined);
  const appWindow = useRef<any>(null);

  useEffect(() => {
    import('@tauri-apps/api/window').then(({ Window }) => {
      appWindow.current = Window;
    });
  }, []);

  useEffect(() => {
    invoke('on_startup').then((val) => {
      if (val === 'log_in') {
        setLayout('login');
        appWindow.current?.show();
      } else {
        invoke('fetch_streamers');
        appWindow.current?.hide();
      }
    });
    let unlistenLoggedIn: UnlistenFn;

    listen('logged_in', (_event) => {
      setLayout('list');
      setTimeout(() => {
        appWindow.current?.hide();
      }, 1000);
    }).then((fn) => {
      unlistenLoggedIn = fn;
    });

    if (!hasCheckedUpdate.current) {
      hasCheckedUpdate.current = true;
      setTimeout(() => {
        checkForUpdate(false).catch((e) => console.error('Update check error: ', e));
      }, 600);
    }
    return () => {
      unlistenLoggedIn && unlistenLoggedIn();
    };
  }, []);

  return (
    <div className="h-[100%] w-full dark:bg-[#26262c] bg-[#efeff1] flex flex-col">
      <Menu />
      {version.current && import.meta.env.DEV && <p>{version.current}</p>}
      <Separator className="mt-1 mb-2" />
      {layout === 'login' ? <LogIn /> : layout === 'list' && <List setLoading={setLoading} loading={loading} />}
    </div>
  );
};
