import { useEffect, useState } from 'react';
import './App.css';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { LogIn } from './views/LogIn';
import { List } from './views/List';
import { Menu } from './components/Menu';
import { Separator } from '@/components/ui/separator';

export const App = () => {
  const [layout, setLayout] = useState<string>('list');
  const [loading, setLoading] = useState<boolean>(true);
  useEffect(() => {
    invoke('on_startup').then((val) => {
      if (val === 'log_in') {
        setLayout('login');
      } else {
        invoke('fetch_streamers');
      }
    });
    let unlistenLoggedIn: UnlistenFn;

    listen('logged_in', (_event) => {
      setLayout('list');
    }).then((fn) => {
      unlistenLoggedIn = fn;
    });
    return () => {
      unlistenLoggedIn && unlistenLoggedIn();
    };
  }, []);

  return (
    <div className="h-[100%] w-full dark:bg-[#26262c] bg-[#efeff1] flex flex-col">
      <Menu />
      <Separator className="mt-1 mb-2" />
      {layout === 'login' ? <LogIn /> : layout === 'list' && <List setLoading={setLoading} loading={loading} />}
    </div>
  );
};
