import SimpleBar from 'simplebar-react';
import { useTheme } from '../ThemeProvider';
import './ThemedSimpleBar.css';

interface ThemedSimpleBarProps {
  children: React.ReactNode;
  className?: string;
  style?: React.CSSProperties;
  autoHide?: boolean;
}

export const ThemedSimpleBar: React.FC<ThemedSimpleBarProps> = ({
  children,
  className = '',
  style,
  autoHide = true,
}) => {
  const { theme } = useTheme();

  return (
    <SimpleBar
      className={`themed-simplebar ${theme} ${className}`}
      style={{ height: '100%', width: '100%', ...style }}
      autoHide={autoHide}
    >
      {children}
    </SimpleBar>
  );
};
