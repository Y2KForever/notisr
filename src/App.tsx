import { useEffect, useState } from 'react';
import './App.css';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { LogIn } from './views/LogIn';

export const App = () => {
  const [layout, setLayout] = useState<string>('list');
  useEffect(() => {
    invoke('on_startup', {
      fileName: 'config.json',
    }).then((val) => {
      if (val === 'log_in') {
        setLayout('login');
      }
    });
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn;
    listen('logged_in', (_event) => {
      setLayout('list');
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten && unlisten();
    };
  });

  return (
    <>
      {layout === 'login' ? (
        <LogIn />
      ) : (
        layout === 'list' && (
          <>
            <p>List view</p>
          </>
        )
      )}
    </>
  );
};
